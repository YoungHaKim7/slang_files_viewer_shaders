// Vendored from the Slang Playground:
// https://github.com/shader-slang/slang-playground (engine/slang-compilation-engine/src/slang/rendering.slang)
//
// Defines the screen-sized output texture and the drawPixel() helper that
// playground compute shaders call to emit pixels.

import playground;

[format("rgba8")]
[playground::BLACK_SCREEN(1.0, 1.0)]
WTexture2D outputTexture;

void drawPixel(uint2 location, IFunc<float4, int2> renderFunction)
{
    uint width = 0;
    uint height = 0;
    outputTexture.GetDimensions(width, height);

    float4 color = renderFunction(int2(width, height));

    if (location.x >= width || location.y >= height)
        return;

    outputTexture.Store(location, color);
}
