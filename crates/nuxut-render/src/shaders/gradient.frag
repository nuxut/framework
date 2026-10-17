in vec2 v_local;
in vec2 v_uv;
flat in vec2 v_half;
flat in vec4 v_radii;
flat in vec4 v_tl;
flat in vec4 v_tr;
flat in vec4 v_br;
flat in vec4 v_bl;

out vec4 o_color;

void main() {
    vec2 uv = clamp(v_uv, 0.0, 1.0);
    // Köşe renklerinin bilineer karışımı (premultiplied uzayda).
    vec4 color = mix(mix(v_tl, v_tr, uv.x), mix(v_bl, v_br, uv.x), uv.y);
    float d = sd_round_box(v_local, v_half, v_radii);
    o_color = color * clamp(0.5 - d, 0.0, 1.0);
}
