// Dual Kawase yukarı örnekleme: dört eksen (×1) + dört köşegen (×2), /12.
// Kenar dışı örnekler aynalanır (aşağı örneklemedeki gerekçe, t050).
uniform sampler2D u_tex;
uniform vec2 u_half_pixel;   // 0.5 / kaynak boyutu
uniform float u_offset;

in vec2 v_uv;
out vec4 o_color;

vec2 mirror_uv(vec2 uv) {
    return clamp(1.0 - abs(1.0 - abs(uv)), 0.0, 1.0);
}

vec4 tap(vec2 uv) {
    return texture(u_tex, mirror_uv(uv));
}

void main() {
    vec2 o = u_half_pixel * u_offset;
    vec4 sum = tap(v_uv + vec2(-o.x * 2.0, 0.0));
    sum += tap(v_uv + vec2(-o.x, o.y)) * 2.0;
    sum += tap(v_uv + vec2(0.0, o.y * 2.0));
    sum += tap(v_uv + vec2(o.x, o.y)) * 2.0;
    sum += tap(v_uv + vec2(o.x * 2.0, 0.0));
    sum += tap(v_uv + vec2(o.x, -o.y)) * 2.0;
    sum += tap(v_uv + vec2(0.0, -o.y * 2.0));
    sum += tap(v_uv + vec2(-o.x, -o.y)) * 2.0;
    o_color = sum / 12.0;
}
