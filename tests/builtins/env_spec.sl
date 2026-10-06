# Environment variables: getenv / hasenv, the .env.test file the runner loads,
# and the SEC-033 removals. setenv / unsetenv wrapped the unsafe
# `std::env::set_var` and let one worker thread mutate process env while others
# read it; dotenv / dotenv! are replaced by loading `.env` and `.env.{APP_ENV}`
# once at single-threaded boot. The names stay registered so old code gets a
# clear migration error instead of `undefined variable`.

describe("reading the environment") do
  test("getenv returns nil and hasenv false for a missing variable") do
    assert_null(getenv("SOLI_DEFINITELY_NOT_SET_12345"))
    assert_not(hasenv("SOLI_DEFINITELY_NOT_SET_12345"))
  end

  test("getenv and hasenv see a variable the process has") do
    assert(hasenv("PATH"))
    assert_contains(getenv("PATH"), "/")
  end

  test("the runner loads .env.test into the environment") do
    assert_eq(getenv("SOLI_FEATURE_FF_TEST_ON"), "1")
    assert_eq(getenv("SOLI_FEATURE_FF_TEST_OFF"), "0")
  end

  test("getenv expects a string name") do
    assert_raises("getenv() expects string name, got int") do
      getenv(5)
    end
  end
end

describe("SEC-033 removals") do
  test("setenv raises the migration error") do
    assert_raises("setenv() has been removed (SEC-033)") do
      setenv("SOLI_TEST_VAR", "test_value")
    end
  end

  test("unsetenv raises the migration error") do
    assert_raises("unsetenv() has been removed (SEC-033)") do
      unsetenv("SOLI_TEST_REMOVE")
    end
  end

  test("dotenv raises the migration error") do
    message = assert_raises("dotenv() has been removed (SEC-033)") do
      dotenv("tests/fixtures/.env.test")
    end
    assert_contains(message, "`.env` and `.env.{APP_ENV}` are auto-loaded")
  end

  test("dotenv! raises the migration error") do
    assert_raises("dotenv!() has been removed (SEC-033)") do
      dotenv!("tests/fixtures/.env.test")
    end
  end
end
