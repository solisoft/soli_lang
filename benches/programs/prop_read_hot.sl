// Hot property reads — exercises the property-read inline cache:
// the same field is read at the same AST sites millions of times on a
// monomorphic receiver.
class Point {
    x: Int;
    y: Int;

    new(x: Int, y: Int) {
        this.x = x;
        this.y = y;
    }
}

let p = new Point(3, 4);
let sum = 0;
let i = 0;
while (i < 20000) {
    sum = sum + p.x + p.y + p.x + p.y;
    i = i + 1;
}
let result = sum;
