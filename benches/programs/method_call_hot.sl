// Hot monomorphic method calls — exercises the method-call inline cache
// and the bound-method construction cost (method body deep clone).
class Vec2 {
    x: Int;
    y: Int;

    new(x: Int, y: Int) {
        this.x = x;
        this.y = y;
    }

    fn dot(ox: Int, oy: Int) -> Int {
        return this.x * ox + this.y * oy;
    }

    fn norm2() -> Int {
        return this.dot(this.x, this.y);
    }
}

let v = new Vec2(3, 4);
let sum = 0;
let i = 0;
while (i < 20000) {
    sum = sum + v.norm2() + v.dot(1, 2);
    i = i + 1;
}
let result = sum;
