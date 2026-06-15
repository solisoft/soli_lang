class Animal {
    name: String;

    new(name: String) {
        # TODO: assign this.name
    }

    def greet {
        # TODO
        return "";
    }
}

class Dog extends Animal {
    breed: String;

    new(name: String, breed: String) {
        # TODO: super(name), this.breed = breed
    }

    def greet {
        # TODO: super + breed suffix
        return "";
    }
}
