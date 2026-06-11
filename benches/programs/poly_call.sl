// Polymorphic call site — the same AST site dispatches over several
// receiver classes. Exercises the polymorphic tier of the inline cache
// (and the megamorphic fallback once >4 classes hit one site).
class Circle {
    r: Int;
    new(r: Int) { this.r = r; }
    fn area() -> Int { return 3 * this.r * this.r; }
}

class Square {
    s: Int;
    new(s: Int) { this.s = s; }
    fn area() -> Int { return this.s * this.s; }
}

class Rect {
    w: Int;
    h: Int;
    new(w: Int, h: Int) { this.w = w; this.h = h; }
    fn area() -> Int { return this.w * this.h; }
}

let shapes = [new Circle(2), new Square(3), new Rect(2, 5), new Circle(4), new Square(1)];
let total = 0;
let i = 0;
while (i < 4000) {
    for shape in shapes {
        total = total + shape.area();
    }
    i = i + 1;
}
let result = total;
