// Blob alanı (ADR-0008): dolu alan = serbest alan H'nin dışı. H'nin sınırı CPU'da kesin
// hesaplanmış doğru parçaları ve y'ye göre tekdüze yay parçalarıdır (`blob::encode`).
// Her ilkel iki RGBA32F dokusu:
//   0: yapıştırılmış uçlar (başlangıç.xy, bitiş.xy) — komşu ilkellerde birebir aynı
//   1: yaysa (merkez.xy, yarıçap, taraf ±1: yarım çemberin x'e göre yanı), doğru parçasıysa 0
// İşaretli uzaklık = en yakın ilkele uzaklık; içerisi yatay ışın çift-tek kuralıyla (yarı açık
// kural paylaşılan uçları çift saymaz).

#define PRIM_ROW 256

uniform highp sampler2D u_prims;
uniform int u_count;
uniform vec4 u_fill;          // premultiplied
uniform vec4 u_shadow;        // premultiplied; a = 0 → gölge yok
uniform vec3 u_shadow_geom;   // offset.xy, sigma

in vec2 v_pos;
out vec4 o_color;

vec4 prim(int i, int k) {
    int t = i * 2 + k;
    return texelFetch(u_prims, ivec2(t % (PRIM_ROW * 2), t / (PRIM_ROW * 2)), 0);
}

// Dolu alanın işaretli uzaklığı: dolu yerde negatif.
float field(vec2 p) {
    float dmin = 1e10;
    bool inside = false;
    for (int i = 0; i < u_count; i++) {
        vec4 ends = prim(i, 0);
        vec4 arc = prim(i, 1);
        vec2 s = ends.xy;
        vec2 e = ends.zw;
        bool crosses = (s.y > p.y) != (e.y > p.y);
        float d;
        if (arc.z == 0.0) {
            vec2 se = e - s;
            float h = clamp(dot(p - s, se) / max(dot(se, se), 1e-12), 0.0, 1.0);
            d = length(p - s - se * h);
            if (crosses && s.x + (p.y - s.y) * (e.x - s.x) / (e.y - s.y) > p.x) {
                inside = !inside;
            }
        } else {
            vec2 c = arc.xy;
            float r = arc.z;
            float side = arc.w;
            vec2 dp = p - c;
            float l = length(dp);
            vec2 q = dp * (r / max(l, 1e-6));
            float qy = c.y + q.y;
            if (q.x * side >= 0.0 && qy >= min(s.y, e.y) && qy <= max(s.y, e.y)) {
                d = abs(l - r);
            } else {
                d = min(length(p - s), length(p - e));
            }
            float dy = p.y - c.y;
            if (crosses && c.x + side * sqrt(max(r * r - dy * dy, 0.0)) > p.x) {
                inside = !inside;
            }
        }
        dmin = min(dmin, d);
    }
    return inside ? dmin : -dmin;
}

void main() {
    float d = field(v_pos);
    // Gradyan boyuyla normalize: kesin uzaklıkta |∇d| = 1; orta eksende de 1 fiziksel piksel AA.
    float g = length(vec2(dFdx(d), dFdy(d)));
    float cov = clamp(0.5 - d / max(g, 0.5), 0.0, 1.0);
    vec4 color = u_fill * cov;
    if (u_shadow.a > 0.0 && cov < 1.0) {
        float ds = field(v_pos - u_shadow_geom.xy);
        color += u_shadow * blurred_coverage(ds, u_shadow_geom.z) * (1.0 - cov);
    }
    o_color = color;
}
