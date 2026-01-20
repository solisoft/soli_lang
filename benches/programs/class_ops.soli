// Class operations - tests object creation and method dispatch
class Counter {
    private count: Int;

    new() {
        this.count = 0;
    }

    fn increment() -> Void {
        this.count = this.count + 1;
    }

    fn add(n: Int) -> Void {
        this.count = this.count + n;
    }

    fn getCount() -> Int {
        return this.count;
    }
}

let counter = new Counter();
let i = 0;
while (i < 1000) {
    counter.increment();
    i = i + 1;
}
let result = counter.getCount();
