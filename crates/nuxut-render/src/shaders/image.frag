uniform sampler2D u_tex;    // premultiplied RGBA
uniform vec4 u_uv;
uniform vec4 u_radii;
uniform float u_opacity;
uniform vec4 u_tint;        // premultiplied; a == 0 → boyama yok

in vec2 v_local;
in vec2 v_uv;
flat in vec2 v_half;

out vec4 o_color;

void main() {
    float d = sd_round_box(v_local, v_half, u_radii);
    float coverage = clamp(0.5 - d, 0.0, 1.0);
    vec2 uv = clamp(v_uv, min(u_uv.xy, u_uv.zw), max(u_uv.xy, u_uv.zw));
    vec4 texel = texture(u_tex, uv);
    // Boyama: doku yalnızca maske (alfa) olarak kullanılır.
    vec4 color = u_tint.a > 0.0 ? u_tint * texel.a : texel;
    o_color = color * (u_opacity * coverage);
}
