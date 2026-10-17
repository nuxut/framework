// Blob alanının serbest alandan en az SOLID_MARGIN uzak dolu parçası: kapsama kesin 1, gölge
// yok — `blob.frag`'ın aynı piksellerde verdiği renk (u_fill), alan döngüsü olmadan.

uniform vec4 u_fill;  // premultiplied

in vec2 v_pos;
out vec4 o_color;

void main() {
    o_color = u_fill;
}
