// Vertex-ID triangle: no vertex buffer is bound.
// The pipeline is created with no vertex input state, and SV_VertexID
// selects the corner for each of the 3 vertices drawn by cmd_draw(3, ...).

struct VSOutput
{
    float4 position: SV_Position;
    float3 color: COLOR;
};

[shader("vertex")]
VSOutput vertMain(uint vertexID: SV_VertexID)
{
    float2 positions[3] =
    {
        float2(-0.5, -0.5),
        float2(0.5, -0.5),
        float2(0.0, 0.5),
    };

    float3 colors[3] =
    {
        float3(1.0, 0.0, 0.0),
        float3(0.0, 1.0, 0.0),
        float3(0.0, 0.0, 1.0),
    };

    VSOutput output;
    output.position = float4(positions[vertexID], 0.0, 1.0);
    output.color = colors[vertexID];
    return output;
}

[shader("fragment")]
float4 fragMain(VSOutput input) : SV_Target
{
    return float4(input.color, 1.0);
}
