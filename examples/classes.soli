// Object-Oriented Programming in Solilang

// Define an interface
interface Drawable {
    fn draw() -> String;
}

// Base class
class Shape {
    x: Float;
    y: Float;

    new(x: Float, y: Float) {
        this.x = x;
        this.y = y;
    }

    fn getPosition() -> String {
        return "(" + str(this.x) + ", " + str(this.y) + ")";
    }
}

// Derived class with interface implementation
class Circle extends Shape implements Drawable {
    radius: Float;

    new(x: Float, y: Float, radius: Float) {
        this.x = x;
        this.y = y;
        this.radius = radius;
    }

    fn getArea() -> Float {
        return 3.14159 * this.radius * this.radius;
    }

    fn draw() -> String {
        return "Circle at " + this.getPosition() + " with radius " + str(this.radius);
    }
}

class Rectangle extends Shape implements Drawable {
    width: Float;
    height: Float;

    new(x: Float, y: Float, width: Float, height: Float) {
        this.x = x;
        this.y = y;
        this.width = width;
        this.height = height;
    }

    fn getArea() -> Float {
        return this.width * this.height;
    }

    fn draw() -> String {
        return "Rectangle at " + this.getPosition() + " (" + str(this.width) + "x" + str(this.height) + ")";
    }
}

// Create instances
let circle = new Circle(10.0, 20.0, 5.0);
let rect = new Rectangle(0.0, 0.0, 10.0, 5.0);

print("Shapes Demo:");
print(circle.draw());
print("Area:", circle.getArea());

print(rect.draw());
print("Area:", rect.getArea());
