in vec2 v_pos;
flat in vec4 v_a;
flat in vec4 v_b;
flat in vec4 v_color;
flat in vec2 v_param;

out vec4 o_color;

// Yuvarlak uçlu doğru parçası (kapsül).
float sd_segment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

// Yuvarlak uçlu çember yayı: merkeze göre p, yarıçap r, başlangıç ve süpürme (radyan).
float sd_arc(vec2 p, float r, float start, float sweep) {
    float half_sweep = 0.5 * abs(sweep);
    if (half_sweep >= 3.14159265) {
        return abs(length(p) - r);   // tam halka
    }
    // Yayı +x eksenine göre simetrik olacak şekilde döndür.
    float mid = start + 0.5 * sweep;
    float c = cos(mid);
    float s = sin(mid);
    vec2 q = vec2(c * p.x + s * p.y, -s * p.x + c * p.y);
    float ang = atan(q.y, q.x);
    if (abs(ang) <= half_sweep) {
        return abs(length(q) - r);
    }
    vec2 e = r * vec2(cos(half_sweep), sin(half_sweep));
    return min(length(q - e), length(q - vec2(e.x, -e.y)));
}

// Eğri altı alan sütunu: x ∈ [x0, x1) sert (piksel merkezi kuralı; komşu sütunla ek yeri ve çift
// karışım yok), üst kenar eğik doğru (dik uzaklıkla 1 px AA), taban yatay (1 px AA). Opaklık
// geçişi tüm alanın tepesinden (v_b.z) tabana doğru v_b.y çarpanına iner.
float area_coverage(vec2 p) {
    float x0 = v_a.x;
    float x1 = v_a.z;
    float inside_x = step(x0, p.x) * (1.0 - step(x1, p.x));
    float w = max(x1 - x0, 1e-6);
    float slope = (v_a.w - v_a.y) / w;
    float top = v_a.y + (p.x - x0) * slope;
    float base = v_b.x;
    // Taban üst kenarın altındaysa (y aşağı) alan aşağı doğru; değilse yukarı.
    float dir = base >= 0.5 * (v_a.y + v_a.w) ? 1.0 : -1.0;
    float cov_top = clamp(0.5 + (p.y - top) * dir * inversesqrt(1.0 + slope * slope), 0.0, 1.0);
    float cov_base = clamp(0.5 + (base - p.y) * dir, 0.0, 1.0);
    float span = base - v_b.z;
    float g = abs(span) > 1e-3 ? clamp((p.y - v_b.z) / span, 0.0, 1.0) : 0.0;
    return inside_x * cov_top * cov_base * mix(1.0, v_b.y, g);
}

void main() {
    if (v_param.y > 1.5) {
        o_color = v_color * area_coverage(v_pos);
        return;
    }
    float d = v_param.y > 0.5
        ? sd_arc(v_pos - v_a.xy, v_a.z, v_b.x, v_b.y)
        : sd_segment(v_pos, v_a.xy, v_a.zw);
    d -= v_param.x;
    o_color = v_color * clamp(0.5 - d, 0.0, 1.0);
}
