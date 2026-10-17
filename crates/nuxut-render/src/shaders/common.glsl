// Ortak SDF yardımcıları. Tüm mesafeler fiziksel piksel; y aşağı.

// Köşe başına yarıçaplı yuvarlatılmış kutu. p kutu merkezine göre;
// r = (sol-üst, sağ-üst, sağ-alt, sol-alt).
float sd_round_box(vec2 p, vec2 half_size, vec4 r) {
    float rad = p.x > 0.0 ? (p.y > 0.0 ? r.z : r.y) : (p.y > 0.0 ? r.w : r.x);
    vec2 q = abs(p) - half_size + rad;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - rad;
}

// Hata fonksiyonu (Abramowitz–Stegun 7.1.26, |hata| < 1.5e-7).
float erf_approx(float x) {
    float s = sign(x);
    float a = abs(x);
    float t = 1.0 / (1.0 + 0.3275911 * a);
    float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
                     + 0.254829592) * t * exp(-a * a);
    return s * y;
}

// İşaretli mesafesi d olan kenarın σ ile Gauss bulanıklaştırılmış kapsaması.
float blurred_coverage(float d, float sigma) {
    return 0.5 - 0.5 * erf_approx(d / (sigma * 1.41421356));
}
