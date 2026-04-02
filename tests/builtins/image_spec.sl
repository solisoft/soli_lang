// ============================================================================
// Image Test Suite
// ============================================================================

describe("Image Class", fn() {
    test("load image from file path", fn() {
        let img = Image.new("tests/fixtures/test.png");
        assert(img != null);
        assert(img.width > 0);
        assert(img.height > 0);
    });

    test("image has width and height properties", fn() {
        let img = Image.new("tests/fixtures/test.png");
        assert(img.width > 0);
        assert(img.height > 0);
    });

    test("thumbnail creates smaller image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let thumb = img.thumbnail(50);
        assert(thumb != null);
        assert(thumb.width <= 50);
        assert(thumb.height <= 50);
    });

    test("grayscale returns image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let gray = img.grayscale();
        assert(gray != null);
    });

    test("resize returns modified image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let resized = img.resize(100, 100);
        assert(resized != null);
    });

    test("chain multiple operations", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let result = img.resize(200, 200).grayscale();
        assert(result != null);
    });

    test("flip_horizontal returns image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let flipped = img.flip_horizontal();
        assert(flipped != null);
    });

    test("flip_vertical returns image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let flipped = img.flip_vertical();
        assert(flipped != null);
    });

    test("rotate90 returns image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let rotated = img.rotate90();
        assert(rotated != null);
    });

    test("blur returns image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let blurred = img.blur(5.0);
        assert(blurred != null);
    });

    test("brightness returns image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let bright = img.brightness(10);
        assert(bright != null);
    });

    test("contrast returns image", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let contrasted = img.contrast(1.5);
        assert(contrasted != null);
    });

    test("to_buffer returns string", fn() {
        let img = Image.new("tests/fixtures/test.png");
        let buffer = img.to_buffer();
        assert(buffer != null);
        assert(buffer != "");
    });
});
