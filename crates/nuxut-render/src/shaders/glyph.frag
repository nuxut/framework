uniform sampler2D u_mask;
uniform sampler2D u_color;
// Açık renkli metin için kapsama üssü (≤ 1 kalınlaştırır). Doğrusal olmayan (sRGB) karışımda
// koyu zemindeki açık metin ince ve soluk görünür; kapsamayı metnin parlaklığıyla orantılı
// kaldırmak bunu dengeler. Koyu metin (açık zeminde) etkilenmez.
uniform float u_light_gamma;

in vec2 v_uv;
flat in vec4 v_color;
flat in float v_kind;

out vec4 o_color;

void main() {
    if (v_kind > 0.5) {
        // Renkli glyph (emoji): atlas premultiplied; yalnızca opaklık uygulanır.
        o_color = texture(u_color, v_uv) * v_color.a;
    } else {
        float a = texture(u_mask, v_uv).r;
        vec3 straight = v_color.a > 0.0 ? v_color.rgb / v_color.a : vec3(0.0);
        float lum = dot(straight, vec3(0.2126, 0.7152, 0.0722));
        a = pow(a, mix(1.0, u_light_gamma, smoothstep(0.35, 0.85, lum)));
        o_color = v_color * a;
    }
}
