describe("Test", fn() {
    test("test1", fn() {
        let name = "World";
        let greeting = "Hello #{name}!";
        assert_eq(greeting, "Hello World!");
    });
    
    test("test2", fn() {
        let a = 2;
        let b = 3;
        let result = "Sum is #{a + b}";
        assert_eq(result, "Sum is 5");
    });

    test("test3", fn() {
        let first = "John";
        let last = "Doe";
        let full = "#{first} #{last}";
        assert_eq(full, "John Doe");
    });

    test("test4", fn() {
        let x = 10;
        let msg = "Double is #{x * 2}";
        assert_eq(msg, "Double is 20");
    });

    test("test5", fn() {
        let text = "hello";
        let result = "Uppercase: #{text.upcase()}";
        assert_eq(result, "Uppercase: HELLO");
    });

    test("test6", fn() {
        let names = ["Alice", "Bob"];
        let result = "First: #{names[0]}";
        assert_eq(result, "First: Alice");
    });

    test("test7", fn() {
        let person = {name: "Charlie"};
        let result = "Name: #{person["name"]}";
        assert_eq(result, "Name: Charlie");
    });
});
