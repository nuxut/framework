uniform vec2 u_viewport;

layout(location = 0) in vec4 i_a;      // doğru: p0.xy, p1.xy | yay: merkez.xy, yarıçap, - | alan: üst kenar x0, y0, x1, y1
layout(location = 1) in vec4 i_b;      // yay: başlangıç, süpürme (radyan), -, - | alan: taban y, tabandaki opaklık, geçiş tepesi y, -
layout(location = 2) in vec4 i_color;  // premultiplied
layout(location = 3) in vec2 i_param;  // yarı kalınlık (fiziksel px), tür (0 = doğru, 1 = yay, 2 = alan)

out vec2 v_pos;
flat out vec4 v_a;
flat out vec4 v_b;
flat out vec4 v_color;
flat out vec2 v_param;

void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    float pad = i_param.x + 1.0;   // yuvarlak uç + 1 px AA payı
    vec2 p;
    if (i_param.y > 1.5) {
        // Alan sütunu: x yarı açık [x0, x1) (sütunlar ek yersiz birleşir), y üst kenar ile taban arası + 1 px AA.
        float y0 = min(min(i_a.y, i_a.w), i_b.x) - 1.0;
        float y1 = max(max(i_a.y, i_a.w), i_b.x) + 1.0;
        p = vec2(mix(i_a.x - 1.0, i_a.z + 1.0, corner.x), mix(y0, y1, corner.y));
    } else if (i_param.y > 0.5) {
        // Yay: tüm çemberin sınır kutusu (tek dörtgen).
        float r = i_a.z + pad;
        p = i_a.xy + mix(vec2(-r), vec2(r), corner);
    } else {
        vec2 d = i_a.zw - i_a.xy;
        float len = length(d);
        vec2 dir = len > 1e-4 ? d / len : vec2(1.0, 0.0);
        vec2 nrm = vec2(-dir.y, dir.x);
        vec2 along = mix(i_a.xy - dir * pad, i_a.zw + dir * pad, corner.x);
        p = along + nrm * mix(-pad, pad, corner.y);
    }
    v_pos = p;
    v_a = i_a;
    v_b = i_b;
    v_color = i_color;
    v_param = i_param;
    vec2 ndc = p / u_viewport * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
