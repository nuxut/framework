uniform vec2 u_viewport;

layout(location = 0) in vec2 a_pos;  // fiziksel piksel

out vec2 v_pos;

void main() {
    v_pos = a_pos;
    vec2 ndc = a_pos / u_viewport * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
