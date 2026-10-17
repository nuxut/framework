uniform vec2 u_viewport;

layout(location = 0) in vec4 i_dest;   // x, y, w, h (fiziksel piksel)
layout(location = 1) in vec4 i_uv;     // u0, v0, u1, v1
layout(location = 2) in vec4 i_color;  // premultiplied
layout(location = 3) in float i_kind;  // 0 = maske, 1 = renkli

out vec2 v_uv;
flat out vec4 v_color;
flat out float v_kind;

void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    vec2 p = i_dest.xy + corner * i_dest.zw;
    v_uv = mix(i_uv.xy, i_uv.zw, corner);
    v_color = i_color;
    v_kind = i_kind;
    vec2 ndc = p / u_viewport * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
