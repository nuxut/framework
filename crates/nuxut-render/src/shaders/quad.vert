uniform vec2 u_viewport;

layout(location = 0) in vec4 i_rect;    // x, y, w, h
layout(location = 1) in vec4 i_radii;
layout(location = 2) in vec4 i_fill;
layout(location = 3) in vec4 i_border;
layout(location = 4) in vec4 i_params;  // border_width, sigma, pad, -

out vec2 v_local;          // şekil merkezine göre konum
flat out vec2 v_half;
flat out vec4 v_radii;
flat out vec4 v_fill;
flat out vec4 v_border;
flat out vec2 v_params;

void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    float pad = i_params.z;
    vec2 p = i_rect.xy - pad + corner * (i_rect.zw + 2.0 * pad);
    v_half = i_rect.zw * 0.5;
    v_local = p - (i_rect.xy + v_half);
    v_radii = i_radii;
    v_fill = i_fill;
    v_border = i_border;
    v_params = i_params.xy;
    vec2 ndc = p / u_viewport * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
