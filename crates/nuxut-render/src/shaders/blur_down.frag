// Dual Kawase aşağı örnekleme: merkez ×4 + dört köşegen, /8.
// Kenar dışı örnekler aynalanır (bkz. mirror_uv): kenara kıstırma en dış satırı/sütunu ve köşe
// texelini her seviyede dışarı uzatıyordu; kenardaki ince çizgi ya da köşedeki küçük leke bulanık
// görüntüde fazla ağırlık alıyordu (t050).
uniform sampler2D u_tex;
uniform vec2 u_half_pixel;   // 0.5 / kaynak boyutu
uniform float u_offset;

in vec2 v_uv;
out vec4 o_color;

// 0 ve 1 kenarlarına göre ayna; [-1, 2] aralığında kesin, sürekli (türev sıçraması yok), dışı
// kenara kıstırılır (yalnızca 1–2 texellik en küçük seviyelerde olur).
vec2 mirror_uv(vec2 uv) {
    return clamp(1.0 - abs(1.0 - abs(uv)), 0.0, 1.0);
}

vec4 tap(vec2 uv) {
    return texture(u_tex, mirror_uv(uv));
}

void main() {
    vec2 o = u_half_pixel * u_offset;
    vec4 sum = texture(u_tex, v_uv) * 4.0;
    sum += tap(v_uv - o);
    sum += tap(v_uv + o);
    sum += tap(v_uv + vec2(o.x, -o.y));
    sum += tap(v_uv - vec2(o.x, -o.y));
    o_color = sum / 8.0;
}
