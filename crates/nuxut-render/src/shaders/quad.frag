in vec2 v_local;
flat in vec2 v_half;
flat in vec4 v_radii;
flat in vec4 v_fill;
flat in vec4 v_border;
flat in vec2 v_params;

out vec4 o_color;

void main() {
    float d = sd_round_box(v_local, v_half, v_radii);
    float sigma = v_params.y;
    if (sigma > 0.0) {
        o_color = v_fill * blurred_coverage(d, sigma);
        return;
    }
    float outer = clamp(0.5 - d, 0.0, 1.0);
    float bw = v_params.x;
    if (bw > 0.0) {
        float inner = clamp(0.5 - (d + bw), 0.0, 1.0);
        o_color = v_fill * inner + v_border * (outer - inner);
    } else {
        o_color = v_fill * outer;
    }
}
