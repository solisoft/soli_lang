class Animal {
    name: String;

    new(name: String) {
        this.name = name;
    }

    def greet {
        return "hi, I'm " + this.name;
    }
}

class Dog extends Animal {
    breed: String;

    new(name: String, breed: String) {
        super(name);
        this.breed = breed;
    }

    def greet {
        return super.greet + " (a " + this.breed + ")";
    }
}
