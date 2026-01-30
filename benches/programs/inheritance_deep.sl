// Deep inheritance benchmark - tests method lookup with inheritance chain
// This tests the optimized flattened method cache

class Base {
    fn baseMethod() -> Int {
        return 1;
    }
}

class Level1 extends Base {
    fn level1Method() -> Int {
        return 2;
    }
}

class Level2 extends Level1 {
    fn level2Method() -> Int {
        return 3;
    }
}

class Level3 extends Level2 {
    fn level3Method() -> Int {
        return 4;
    }
}

class Level4 extends Level3 {
    fn level4Method() -> Int {
        return 5;
    }
}

// Final class with deep inheritance
class DeepClass extends Level4 {
    fn deepMethod() -> Int {
        return this.baseMethod() + this.level1Method() + this.level2Method() + this.level3Method() + this.level4Method();
    }
}

let obj = new DeepClass();
let sum = 0;
let i = 0;
while (i < 1000) {
    // Each iteration calls methods from all levels of inheritance
    sum = sum + obj.deepMethod();
    i = i + 1;
}
let result = sum;
