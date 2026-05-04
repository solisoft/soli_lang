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

describe("Image Plan (parallel processing)", fn() {
    test("Image.plan returns an ImagePlan instance", fn() {
        let p = Image.plan("tests/fixtures/test.png");
        assert(p != null);
        assert_eq(p.src(), "tests/fixtures/test.png");
        assert_eq(p.ops_count(), 0);
    });

    test("plan chain records ops without executing", fn() {
        let p = Image.plan("tests/fixtures/missing.png")
          .grayscale()
          .rotate90()
          .resize(100, 100);
        # If anything were executed eagerly, the missing-file path would have thrown.
        assert_eq(p.ops_count(), 3);
    });

    test("plan.run() executes and returns Image when no save_to", fn() {
        let img = Image.plan("tests/fixtures/test.png")
          .grayscale()
          .rotate90()
          .run();
        assert(img != null);
        assert(img.width > 0);
    });

    test("plan.save_to + run writes the file", fn() {
        let out = "tests/fixtures/_plan_run_out.png";
        let ok = Image.plan("tests/fixtures/test.png")
          .grayscale()
          .save_to(out)
          .run();
        assert_eq(ok, true);
        assert(File.exists(out));
        File.delete(out);
    });

    test("Image.process_all runs plans in parallel and saves", fn() {
        let out_a = "tests/fixtures/_plan_a.png";
        let out_b = "tests/fixtures/_plan_b.png";
        let results = Image.process_all([
          Image.plan("tests/fixtures/test.png").grayscale().save_to(out_a),
          Image.plan("tests/fixtures/test.png").rotate90().save_to(out_b),
        ]);
        assert_eq(len(results), 2);
        assert_eq(results[0], true);
        assert_eq(results[1], true);
        assert(File.exists(out_a));
        assert(File.exists(out_b));
        File.delete(out_a);
        File.delete(out_b);
    });

    test("Image.process_all returns Image instances when no save_to", fn() {
        let results = Image.process_all([
          Image.plan("tests/fixtures/test.png").grayscale(),
          Image.plan("tests/fixtures/test.png").rotate90().resize(50, 50),
        ]);
        assert_eq(len(results), 2);
        assert(results[0].width > 0);
        assert(results[1].width > 0);
    });

    test("Image.process_all reports per-plan errors as hash", fn() {
        let results = Image.process_all([
          Image.plan("tests/fixtures/test.png").grayscale(),
          Image.plan("tests/fixtures/does_not_exist.png").grayscale(),
        ]);
        assert_eq(len(results), 2);
        assert(results[0] != null);
        assert(results[1].error != null);
    });
});
