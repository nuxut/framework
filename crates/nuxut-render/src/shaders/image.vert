uniform vec2 u_viewport;
uniform vec4 u_rect;    // x, y, w, h (fiziksel piksel)
uniform vec4 u_uv;      // u0, v0, u1, v1

out vec2 v_local;
out vec2 v_uv;
flat out vec2 v_half;

void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    // 1 px AA payı; doku koordinatı payda da doğrusal uzar (kenarda clamp).
    vec2 p = u_rect.xy - 1.0 + corner * (u_rect.zw + 2.0);
    v_half = u_rect.zw * 0.5;
    v_local = p - (u_rect.xy + v_half);
    vec2 t = (p - u_rect.xy) / max(u_rect.zw, vec2(1e-3));
    v_uv = mix(u_uv.xy, u_uv.zw, t);
    vec2 ndc = p / u_viewport * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
