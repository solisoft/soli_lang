# LLM and embedding builtins on their offline paths: argument checks, and the
# config error raised before any network call when no API key is set.

describe("llm_generate") do
  test("refuses a prompt that is not a string") do
    assert_raises("llm_generate expects (system, user) strings") do
      llm_generate(42, "user prompt")
    end
  end
end

describe("embed and embed_batch") do
  test("embed refuses input that is not a string") do
    assert_raises("embed expects a text string") do
      embed(42)
    end
  end

  test("embed_batch refuses an array holding a non-string") do
    assert_raises("embed_batch expects an array of strings") do
      embed_batch(["ok", 42])
    end
  end

  context("without SOLI_EMBEDDING_API_KEY") do
    before_each() do
      skip("SOLI_EMBEDDING_API_KEY is set") if hasenv("SOLI_EMBEDDING_API_KEY")
    end

    test("embed fails fast with a config error") do
      assert_raises("embed could not generate an embedding: set SOLI_EMBEDDING_API_KEY") do
        embed("some text")
      end
    end

    test("embed_batch fails fast with a config error") do
      assert_raises("embed_batch could not generate embeddings: set SOLI_EMBEDDING_API_KEY") do
        embed_batch(["a", "b"])
      end
    end
  end
end
