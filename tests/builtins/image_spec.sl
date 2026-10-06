# Image: loading, every transform (asserting the resulting dimensions and
# pixels), encoding formats, writing to disk, and the parallel ImagePlan API.

# A generated 40x30 RGB PNG; tests/fixtures/test.png is only 1x1.
# Its colors run left to right only, so it is symmetric top to bottom.
const TEST_PNG_B64 = "iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAIAAADRv8uKAAAAVUlEQVR4nGNgYOMRkpBT0TIws3Hy8AuJSkjLKaqoa+" +
  "maMG3OohXrtuw6cOzMpRv3nryiurpRi0ctHrV41OJRi0ctHrV41OJRi0ctHrV41OJRiwevxQDOb22rjHJEuQAAAABJ" +
  "RU5ErkJggg=="
const FIXTURE = "tests/fixtures/test.png"
const MISSING = "tests/fixtures/does_not_exist.png"
const OUT_PATHS = [
  "tests/fixtures/_image_spec_out.png",
  "tests/fixtures/_image_spec_out.jpg",
  "tests/fixtures/_image_spec_a.png",
  "tests/fixtures/_image_spec_b.png"
]

# Base64 signatures of the encoded formats.
const PNG_PREFIX = "iVBOR"
const JPEG_PREFIX = "/9j/"

image = nil

# Several transforms return RGBA pixels whatever the input, so pixel-level
# comparisons are made against an RGBA copy of the same picture.
rgba = nil

describe("Image") do
  before_each() do
    image = Image.from_buffer(TEST_PNG_B64)
    rgba = image.brightness(0)
  end

  after_each() do
    OUT_PATHS.each do |path|
      File.delete(path) if File.exists(path)
    end
  end

  describe("loading") do
    test("Image.new reads a file and reports its dimensions") do
      fixture = Image.new(FIXTURE)
      assert_eq(fixture.class, "Image")
      assert_eq(fixture.width, 1)
      assert_eq(fixture.height, 1)
    end

    test("Image.new keeps the format sniffed from the file") do
      assert(Image.new(FIXTURE).to_buffer.starts_with(PNG_PREFIX))
    end

    test("Image.new raises on a missing file") do
      assert_raises("Failed to open image") do
        Image.new(MISSING)
      end
    end

    test("Image.new raises on a file that is not an image") do
      assert_raises("Failed to decode image") do
        Image.new("tests/fixtures/test.csv")
      end
    end

    test("Image.from_buffer decodes a base64 PNG") do
      assert_eq(image.width, 40)
      assert_eq(image.height, 30)
    end

    test("to_buffer round-trips through from_buffer") do
      copy = Image.from_buffer(image.to_buffer)
      assert_eq(copy.width, 40)
      assert_eq(copy.height, 30)
      assert_eq(copy.to_buffer, image.to_buffer)
    end

    test("Image.from_buffer rejects invalid base64") do
      assert_raises("Failed to decode base64 buffer") do
        Image.from_buffer("!!not base64")
      end
    end

    test("Image.from_buffer rejects bytes that are not an image") do
      assert_raises("image format could not be determined") do
        Image.from_buffer(Base64.encode("hello"))
      end
    end
  end

  describe("resizing") do
    test("resize fits inside the box and keeps the aspect ratio") do
      resized = image.resize(100, 50)
      assert_eq(resized.width, 67)
      assert_eq(resized.height, 50)
    end

    test("resize to a square box is limited by the wider side") do
      resized = image.resize(20, 20)
      assert_eq(resized.width, 20)
      assert_eq(resized.height, 15)
    end

    test("resize requires integer dimensions") do
      assert_raises("Image.resize requires integer width") do
        image.resize("wide", 20)
      end
    end

    test("thumbnail scales the longest side down to the size") do
      thumb = image.thumbnail(20)
      assert_eq(thumb.width, 20)
      assert_eq(thumb.height, 15)
    end

    test("thumbnail also scales a small image up") do
      thumb = image.thumbnail(100)
      assert_eq(thumb.width, 100)
      assert_eq(thumb.height, 75)
    end

    test("transforms return a new image and leave the receiver alone") do
      image.resize(10, 10)
      assert_eq(image.width, 40)
      assert_eq(image.height, 30)
    end
  end

  describe("cropping") do
    test("crop extracts the requested rectangle") do
      cropped = image.crop(0, 0, 10, 20)
      assert_eq(cropped.width, 10)
      assert_eq(cropped.height, 20)
    end

    test("crop is clamped to the image bounds") do
      cropped = image.crop(30, 20, 20, 20)
      assert_eq(cropped.width, 10)
      assert_eq(cropped.height, 10)
    end

    test("crop refuses a negative origin") do
      assert_raises("Image.crop requires non-negative x") do
        image.crop(-1, 0, 2, 2)
      end
    end
  end

  describe("orientation") do
    test("rotate90 swaps width and height") do
      rotated = image.rotate90
      assert_eq(rotated.width, 30)
      assert_eq(rotated.height, 40)
    end

    test("rotate180 keeps the dimensions") do
      rotated = image.rotate180
      assert_eq(rotated.width, 40)
      assert_eq(rotated.height, 30)
    end

    test("rotate270 swaps width and height") do
      rotated = image.rotate270
      assert_eq(rotated.width, 30)
      assert_eq(rotated.height, 40)
    end

    test("four quarter turns give back the original pixels") do
      assert_eq(image.rotate90.rotate90.rotate90.rotate90.to_buffer, image.to_buffer)
    end

    test("rotate180 and rotate270 are two and three quarter turns") do
      assert_eq(image.rotate180.to_buffer, image.rotate90.rotate90.to_buffer)
      assert_eq(image.rotate270.to_buffer, image.rotate90.rotate90.rotate90.to_buffer)
    end

    test("flip_horizontal mirrors the pixels and undoes itself") do
      flipped = image.flip_horizontal
      assert_eq(flipped.width, 40)
      assert_eq(flipped.height, 30)
      assert_ne(flipped.to_buffer, rgba.to_buffer)
      assert_eq(flipped.flip_horizontal.to_buffer, rgba.to_buffer)
    end

    test("flip_vertical leaves a top-to-bottom symmetric picture unchanged") do
      flipped = image.flip_vertical
      assert_eq(flipped.width, 40)
      assert_eq(flipped.height, 30)
      assert_eq(flipped.to_buffer, rgba.to_buffer)
    end

    test("flip_vertical mirrors the pixels and undoes itself") do
      # Turned a quarter, the colors run top to bottom.
      turned = image.rotate90.brightness(0)
      assert_ne(turned.flip_vertical.to_buffer, turned.to_buffer)
      assert_eq(turned.flip_vertical.flip_vertical.to_buffer, turned.to_buffer)
    end
  end

  describe("color and filters") do
    test("grayscale changes the pixels and is idempotent") do
      gray = image.grayscale
      assert_eq(gray.width, 40)
      assert_ne(gray.to_buffer, image.to_buffer)
      assert_eq(gray.grayscale.to_buffer, gray.to_buffer)
    end

    test("invert twice gives back the original pixels") do
      assert_ne(image.invert.to_buffer, image.to_buffer)
      assert_eq(image.invert.invert.to_buffer, image.to_buffer)
    end

    test("hue_rotate by 0 is a no-op and by 90 shifts the colors") do
      assert_eq(image.hue_rotate(0).to_buffer, rgba.to_buffer)
      shifted = image.hue_rotate(90)
      assert_eq(shifted.width, 40)
      assert_eq(shifted.height, 30)
      assert_ne(shifted.to_buffer, rgba.to_buffer)
    end

    test("brightness lightens the pixels") do
      assert_ne(image.brightness(30).to_buffer, rgba.to_buffer)
    end

    test("contrast changes the pixels") do
      assert_ne(image.contrast(30.0).to_buffer, rgba.to_buffer)
    end

    test("blur keeps the dimensions and softens the pixels") do
      blurred = image.blur(2.0)
      assert_eq(blurred.width, 40)
      assert_eq(blurred.height, 30)
      assert_ne(blurred.to_buffer, rgba.to_buffer)
    end

    test("filters require a numeric argument") do
      assert_raises("Image.blur requires number") do
        image.blur("lots")
      end
      assert_raises("Image.brightness requires integer") do
        image.brightness(1.5)
      end
    end

    test("operations chain") do
      result = image.resize(20, 20).grayscale.rotate90
      assert_eq(result.width, 15)
      assert_eq(result.height, 20)
    end
  end

  describe("encoding") do
    test("to_buffer defaults to the source format") do
      assert(image.to_buffer.starts_with(PNG_PREFIX))
    end

    test("format switches the encoder") do
      assert(image.format("jpeg").to_buffer.starts_with(JPEG_PREFIX))
      assert(image.format("webp").to_buffer.starts_with("UklGR"))
      assert(image.format("gif").to_buffer.starts_with("R0lGOD"))
      assert(image.format("bmp").to_buffer.starts_with("Qk"))
    end

    test("format rejects an unknown format") do
      assert_raises("Unsupported format: xyz") do
        image.format("xyz")
      end
    end

    test("quality does not change a PNG") do
      assert_eq(image.quality(40).to_buffer, image.to_buffer)
    end

    test("quality trades size for fidelity in JPEG") do
      jpeg = image.format("jpeg")
      assert_lt(jpeg.quality(10).to_buffer.length, jpeg.quality(95).to_buffer.length)
    end

    test("to_file writes the encoded image to disk") do
      path = OUT_PATHS[0]
      assert_eq(image.grayscale.to_file(path), true)
      reloaded = Image.new(path)
      assert_eq(reloaded.width, 40)
      assert_eq(reloaded.height, 30)
      assert_eq(reloaded.to_buffer, image.grayscale.to_buffer)
    end

    test("to_file picks the encoder from the extension") do
      path = OUT_PATHS[1]
      image.to_file(path)
      assert(Image.new(path).to_buffer.starts_with(JPEG_PREFIX))
    end
  end

  describe("Image.plan (parallel processing)") do
    test("records the source and no ops") do
      plan = Image.plan(FIXTURE)
      assert_eq(plan.class, "ImagePlan")
      assert_eq(plan.src, FIXTURE)
      assert_eq(plan.ops_count, 0)
    end

    test("records ops without executing them") do
      # The file is missing: anything run eagerly would have raised.
      plan = Image.plan(MISSING).grayscale.rotate90.resize(100, 100)
      assert_eq(plan.ops_count, 3)
    end

    test("run without save_to returns the transformed Image") do
      result = Image.plan(FIXTURE).format("jpeg").quality(50).thumbnail(10).run
      assert_eq(result.width, 10)
      assert_eq(result.height, 10)
      assert(result.to_buffer.starts_with(JPEG_PREFIX))
    end

    test("save_to + run writes the file and returns true") do
      path = OUT_PATHS[0]
      assert_eq(Image.plan(FIXTURE).grayscale.save_to(path).run, true)
      assert_eq(Image.new(path).width, 1)
    end

    test("process_all runs plans and saves each one") do
      first_path = OUT_PATHS[2]
      second_path = OUT_PATHS[3]
      results = Image.process_all([
        Image.plan(FIXTURE).grayscale.save_to(first_path),
        Image.plan(FIXTURE).rotate90.save_to(second_path)
      ])
      assert_eq(results, [true, true])
      assert(File.exists(first_path))
      assert(File.exists(second_path))
    end

    test("process_all returns Image instances when nothing is saved") do
      results = Image.process_all([
        Image.plan(FIXTURE).grayscale,
        Image.plan(FIXTURE).rotate90.resize(50, 50)
      ])
      assert_eq(results.length, 2)
      assert_eq(results[0].width, 1)
      assert_eq(results[1].width, 50)
    end

    test("process_all reports a failing plan as an error hash") do
      results = Image.process_all([Image.plan(FIXTURE).grayscale, Image.plan(MISSING).grayscale])
      assert_eq(results[0].width, 1)
      assert_contains(results[1]["error"], "Failed to open image")
    end

    test("process_all refuses something that is not an array") do
      assert_raises("expects array of plans") do
        Image.process_all("plans")
      end
    end
  end
end
