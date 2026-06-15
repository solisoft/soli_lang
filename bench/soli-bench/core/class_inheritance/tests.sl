describe("class_inheritance", fn() {
    test("Animal.greet", fn() {
        let a = new Animal("Alex");
        assert_eq(a.greet, "hi, I'm Alex");
    });

    test("Dog.greet chains super", fn() {
        let d = new Dog("Rex", "lab");
        assert_eq(d.greet, "hi, I'm Rex (a lab)");
    });

    test("Dog inherits name field", fn() {
        let d = new Dog("Rex", "lab");
        assert_eq(d.name, "Rex");
    });

    test("Dog has its own breed", fn() {
        let d = new Dog("Rex", "lab");
        assert_eq(d.breed, "lab");
    });
});
