uniform vec2 u_viewport;

layout(location = 0) in vec4 i_rect;   // x, y, w, h (fiziksel piksel)
layout(location = 1) in vec4 i_radii;
layout(location = 2) in vec4 i_tl;     // köşe renkleri, premultiplied
layout(location = 3) in vec4 i_tr;
layout(location = 4) in vec4 i_br;
layout(location = 5) in vec4 i_bl;

out vec2 v_local;
out vec2 v_uv;
flat out vec2 v_half;
flat out vec4 v_radii;
flat out vec4 v_tl;
flat out vec4 v_tr;
flat out vec4 v_br;
flat out vec4 v_bl;

void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    // 1 px AA payı.
    vec2 p = i_rect.xy - 1.0 + corner * (i_rect.zw + 2.0);
    v_half = i_rect.zw * 0.5;
    v_local = p - (i_rect.xy + v_half);
    v_uv = (p - i_rect.xy) / max(i_rect.zw, vec2(1e-3));
    v_radii = i_radii;
    v_tl = i_tl;
    v_tr = i_tr;
    v_br = i_br;
    v_bl = i_bl;
    vec2 ndc = p / u_viewport * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
