// ============================================================================
// Decimal Type Test Suite
// ============================================================================

describe("Decimal Type", fn() {
    test("decimal literal syntax", fn() {
        let price = 19.99D;
        assert(type(price) == "decimal");
    });

    test("decimal with 2 precision", fn() {
        let amount = 100.00D;
        assert(type(amount) == "decimal");
    });

    test("decimal with 4 precision", fn() {
        let rate = 0.0675D;
        assert(type(rate) == "decimal");
    });

    test("decimal display", fn() {
        let price = 19.99D;
        let s = str(price);
        assert(s == "19.99");
    });

    test("decimal in array", fn() {
        let prices = [19.99D, 29.99D, 9.99D];
        assert(len(prices) == 3);
    });

    test("decimal in hash", fn() {
        let product = {
            "price": 19.99D,
            "name": "Widget"
        };
        assert(product["price"] == 19.99D);
    });

    test("zero decimal", fn() {
        let zero = 0.00D;
        assert(type(zero) == "decimal");
    });
});

describe("Decimal Edge Cases", fn() {
    test("negative decimal", fn() {
        let negative = -10.50D;
        assert(type(negative) == "decimal");
        assert(negative < 0.00D);
    });

    test("very small decimal", fn() {
        let small = 0.000001D;
        assert(type(small) == "decimal");
    });

    test("large decimal", fn() {
        let large = 9999999999.99D;
        assert(type(large) == "decimal");
    });

    test("decimal with all zeros after decimal", fn() {
        let zero_decimal = 100.00D;
        assert(type(zero_decimal) == "decimal");
    });

    test("single digit after decimal", fn() {
        let single = 5.5D;
        assert(type(single) == "decimal");
    });
});

describe("Decimal Conversions", fn() {
    test("decimal to string via str()", fn() {
        let value = 19.99D;
        let s = str(value);
        assert(len(s) > 0);
    });

    test("decimal equality check", fn() {
        let a = 19.99D;
        let b = 19.99D;
        let c = 20.00D;

        assert(a == b);
        assert(a != c);
    });

    test("decimal with type assertion", fn() {
        let value = 99.99D;
        assert(type(value) == "decimal");
    });
});

describe("Decimal in Collections", fn() {
    test("array of decimals", fn() {
        let prices = [
            10.00D,
            20.50D,
            30.75D
        ];
        assert(len(prices) == 3);
        assert(prices[0] < prices[2]);
    });

    test("hash with decimal values", fn() {
        let order = {
            "subtotal": 100.00D,
            "tax": 8.50D,
            "total": 108.50D
        };
        assert(order["subtotal"] < order["total"]);
    });

    test("nested hash with decimals", fn() {
        let product = {
            "pricing": {
                "base": 50.00D,
                "discount": 5.00D,
                "final": 45.00D
            },
            "name": "Widget"
        };
        assert(product["pricing"]["final"] == 45.00D);
    });
});
