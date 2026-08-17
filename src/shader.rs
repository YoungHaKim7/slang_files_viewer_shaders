//! Runtime compilation of a user-supplied .slang file via `slangc`.
//!
//! The viewer takes the shader path as a command-line argument (or the
//! source on stdin), compiles the whole module to SPIR-V in one `slangc`
//! invocation, and inspects the emitted reflection JSON to decide how to
//! display it:
//!
//! - vertex + fragment entry points  -> graphics pipeline
//! - compute entry point             -> playground-style compute pass
//!
//! Playground demos (e.g. the 2D gaussian splatter) rely on a prelude that
//! the web playground injects (`drawPixel`, the screen-sized output texture,
//! the `[playground::...]` attributes). When a file does not compile on its
//! own, the vendored prelude in `assets/playground/` is written next to it
//! and the compile is retried with the matching imports prepended.

use serde_json::Value;
use std::{
    env, fs,
    io::{IsTerminal, Read},
    path::{Path, PathBuf},
    process::Command,
};

const PLAYGROUND_PRELUDE: &str = include_str!("../assets/playground/playground.slang");

const RENDERING_PRELUDE: &str = include_str!("../assets/playground/rendering.slang");

/// Default element count for an unattributed `RWStructuredBuffer<float>`.
///
/// The playground's gaussian-splat demo reads `[playground::RAND(131072)]`;
/// local copies of the demo usually have the attribute stripped, so the
/// viewer fills any unattributed float buffer with this many randoms.
pub const DEFAULT_RAND_COUNT: u32 = 131_072;

/// The .slang file to view: either a path given on the command line or the
/// contents of stdin dumped to disk (slangc only reads files).
pub struct SourceFile {
    /// Shown in the window title.
    pub display_name: String,
    /// Path of the source on disk.
    pub path: PathBuf,
}

/// How a compiled module is displayed.
pub enum RenderMode {
    /// Classic vertex + fragment pair rendered through the render pass.
    Graphics {
        vertex_entry: String,
        fragment_entry: String,
    },
    /// Compute kernel writing pixels through the playground's `drawPixel`.
    Compute {
        entry: String,
        group_size: [u32; 3],
        parameters: Vec<ShaderParam>,
    },
}

/// A module-level shader parameter the viewer must bind.
pub struct ShaderParam {
    pub name: String,
    pub binding: u32,
    pub kind: ParamKind,
    /// Element count from `[playground::RAND(n)]`, if present.
    pub rand_count: Option<u32>,
}

pub enum ParamKind {
    /// `RWStructuredBuffer<float>` filled with uniform randoms.
    RandomFloatBuffer,
    /// Screen-sized storage image the kernel writes to.
    OutputTexture,
    /// Anything the viewer does not know how to supply.
    Unsupported(String),
}

pub struct CompiledShader {
    pub spirv: Vec<u32>,
    pub mode: RenderMode,
}

/// Per-run scratch directory for the prelude files, the stdin dump and the
/// compiler outputs. Lives for the whole process; slangc and the SPIR-V
/// read happen up front.
pub fn create_workdir() -> PathBuf {
    let dir = env::temp_dir().join(format!("slang-viewer-{}", std::process::id()));

    fs::create_dir_all(&dir).expect("create temp workdir");

    dir
}

/// Resolves the shader to view: first command-line argument, else stdin
/// when it is piped (e.g. `viewer < demo.slang`), else usage instructions.
pub fn resolve_source(workdir: &Path) -> SourceFile {
    if let Some(arg) = env::args().nth(1) {
        let path = PathBuf::from(&arg);

        if !path.is_file() {
            eprintln!("error: no such file: {arg}");

            std::process::exit(2);
        }

        let display_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| arg.clone());

        return SourceFile { display_name, path };
    }

    if !std::io::stdin().is_terminal() {
        let mut source = String::new();

        std::io::stdin()
            .read_to_string(&mut source)
            .expect("read shader from stdin");

        if source.trim().is_empty() {
            eprintln!("error: no shader source received on stdin");

            std::process::exit(2);
        }

        let path = workdir.join("stdin.slang");

        fs::write(&path, source).expect("write stdin shader to temp file");

        return SourceFile {
            display_name: "stdin".to_string(),
            path,
        };
    }

    eprintln!("usage: slang_files_viewer_shaders <path/to/shader.slang>");
    eprintln!("       cat shader.slang | slang_files_viewer_shaders");

    std::process::exit(2);
}

/// Compiles the source module and picks a display mode from reflection.
///
/// Exits the process with diagnostics on any user-facing failure.
pub fn compile(workdir: &Path, source: &SourceFile) -> CompiledShader {
    let spirv_path = workdir.join("shader.spv");

    let reflection_path = workdir.join("reflection.json");

    // First attempt: the file as-is.
    let plain = invoke_slangc(&source.path, &spirv_path, &reflection_path, None);

    if let Ok(()) = &plain
        && let Some(compiled) = finish(&spirv_path, &reflection_path)
    {
        return compiled;
    }
    // Compiles standalone but nothing displayable; the scaffold retry
    // below may reveal playground entry points.

    // Retry with the playground prelude available and imported.
    let scaffold_dir = workdir.join("prelude");

    fs::create_dir_all(&scaffold_dir).expect("create prelude dir");

    fs::write(scaffold_dir.join("playground.slang"), PLAYGROUND_PRELUDE)
        .expect("write playground prelude");

    fs::write(scaffold_dir.join("rendering.slang"), RENDERING_PRELUDE)
        .expect("write rendering prelude");

    let scaffold_source =
        with_playground_imports(&fs::read_to_string(&source.path).expect("read shader source"));

    let scaffold_path = workdir.join("with-prelude.slang");

    fs::write(&scaffold_path, scaffold_source).expect("write combined shader source");

    let scaffold = invoke_slangc(
        &scaffold_path,
        &spirv_path,
        &reflection_path,
        Some(&scaffold_dir),
    );

    if let Ok(()) = &scaffold
        && let Some(compiled) = finish(&spirv_path, &reflection_path)
    {
        return compiled;
    }

    // Nothing displayable, or no build succeeded. When the file failed to
    // compile on its own, slangc's plain diagnostics describe the user's
    // actual file best; otherwise explain what the viewer supports.
    match plain {
        Err(plain_stderr) => {
            eprintln!("error: slangc failed to compile {}:", source.display_name);

            eprint!("{plain_stderr}");

            std::process::exit(1);
        }
        Ok(()) => report_not_displayable(),
    }
}

/// Runs slangc on the whole module (no `-entry`, so every entry point is
/// emitted into one SPIR-V file) and requests reflection JSON alongside.
fn invoke_slangc(
    source: &Path,
    spirv_out: &Path,
    reflection_out: &Path,
    include_dir: Option<&Path>,
) -> Result<(), String> {
    let mut command = Command::new("slangc");

    command
        .arg(source)
        .arg("-target")
        .arg("spirv")
        // SPIR-V 1.3 is the newest version Vulkan 1.1 accepts.
        .arg("-profile")
        .arg("spirv_1_3")
        // Keep entry point names (vertMain/fragMain/imageMain) instead of
        // renaming every entry to "main".
        .arg("-fvk-use-entrypoint-name")
        .arg("-reflection-json")
        .arg(reflection_out)
        .arg("-o")
        .arg(spirv_out);

    if let Some(dir) = include_dir {
        command.arg("-I").arg(dir);
    }

    let output = match command.output() {
        Ok(output) => output,
        Err(_) => {
            eprintln!("error: slangc not found on PATH");

            eprintln!("       it ships with the Vulkan SDK (x86_64/bin/slangc)");

            std::process::exit(1);
        }
    };

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// Prepends the playground imports unless the source already has them.
fn with_playground_imports(source: &str) -> String {
    if source.contains("import rendering") {
        source.to_string()
    } else {
        format!("import playground;\nimport rendering;\n\n{source}")
    }
}

/// Loads the SPIR-V and reflection output and selects a display mode.
///
/// Returns `None` when the module compiled but contains nothing the
/// viewer knows how to display.
fn finish(spirv_path: &Path, reflection_path: &Path) -> Option<CompiledShader> {
    let words = read_spirv(spirv_path);

    let reflection: Value =
        serde_json::from_str(&fs::read_to_string(reflection_path).expect("read reflection json"))
            .expect("parse reflection json");

    let entries = reflection["entryPoints"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    (
                        entry["name"].as_str().unwrap_or_default().to_string(),
                        entry["stage"].as_str().unwrap_or_default().to_string(),
                        entry["threadGroupSize"].clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let parameters = parse_parameters(&reflection);

    let vertex = entries
        .iter()
        .find(|(_, stage, _)| stage == "vertex")
        .map(|(name, _, _)| name.clone());

    let fragment = entries
        .iter()
        .find(|(_, stage, _)| stage == "fragment")
        .map(|(name, _, _)| name.clone());

    let compute = entries
        .iter()
        .find(|(_, stage, _)| stage == "compute")
        .cloned();

    // Graphics needs a vertex + fragment pair and must not declare any
    // resource parameters; the viewer supplies no vertex data or textures.
    if let (Some(vertex_entry), Some(fragment_entry)) = (&vertex, &fragment)
        && parameters.is_empty()
    {
        return Some(CompiledShader {
            spirv: words,
            mode: RenderMode::Graphics {
                vertex_entry: vertex_entry.clone(),
                fragment_entry: fragment_entry.clone(),
            },
        });
    }

    if let Some((entry, _, thread_group)) = compute {
        let group_size = [
            thread_group[0].as_u64().unwrap_or(1) as u32,
            thread_group[1].as_u64().unwrap_or(1) as u32,
            thread_group[2].as_u64().unwrap_or(1) as u32,
        ];

        return Some(CompiledShader {
            spirv: words,
            mode: RenderMode::Compute {
                entry,
                group_size,
                parameters,
            },
        });
    }

    None
}

fn parse_parameters(reflection: &Value) -> Vec<ShaderParam> {
    reflection["parameters"]
        .as_array()
        .map(|params| {
            params
                .iter()
                .map(|param| {
                    let name = param["name"].as_str().unwrap_or_default().to_string();

                    let binding = param["binding"]["index"].as_u64().unwrap_or(0) as u32;

                    let base_shape = param["type"]["baseShape"].as_str().unwrap_or("");

                    let access = param["type"]["access"].as_str().unwrap_or("");

                    let is_float =
                        param["type"]["resultType"]["scalarType"].as_str() == Some("float32");

                    let kind = match (base_shape, access) {
                        ("structuredBuffer", "readWrite") if is_float => {
                            ParamKind::RandomFloatBuffer
                        }
                        ("texture2D", "write") => ParamKind::OutputTexture,
                        _ => ParamKind::Unsupported(format!("{base_shape} ({access})")),
                    };

                    // `[playground::RAND(count)]` survives reflection as a
                    // userAttrib named playground_RAND.
                    let rand_count = param["userAttribs"]
                        .as_array()
                        .and_then(|attribs| {
                            attribs
                                .iter()
                                .find(|attrib| attrib["name"].as_str() == Some("playground_RAND"))
                        })
                        .and_then(|attrib| attrib["arguments"].as_array())
                        .and_then(|args| args.first())
                        .and_then(Value::as_u64)
                        .map(|count| count as u32);

                    ShaderParam {
                        name,
                        binding,
                        kind,
                        rand_count,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read_spirv(path: &Path) -> Vec<u32> {
    let bytes = fs::read(path).expect("read compiled SPIR-V");

    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect();

    assert_eq!(
        words.first(),
        Some(&0x0723_0203),
        "slangc did not emit valid SPIR-V"
    );

    words
}

fn report_not_displayable() -> ! {
    eprintln!("error: the module has no entry point this viewer can display.");
    eprintln!("       supported: vertex + fragment stages with no resource parameters,");
    eprintln!("                  or a compute kernel using the playground's drawPixel.");

    std::process::exit(1);
}
