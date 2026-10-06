# The FeatureFlags stdlib module. It ships as a scaffold template; imports are
# sandboxed to the spec's directory, so this runs a fixture copy and checks it
# still byte-matches the template. tests/.env.test sets SOLI_FEATURE_FF_TEST_ON=1
# and SOLI_FEATURE_FF_TEST_OFF=0 for the env-override tests.

import "./_fixtures/feature_flags_fixture.sl"

describe("FeatureFlags fixture") do
  test("byte-matches the shipped scaffold template") do
    template = slurp("src/scaffold/templates/feature_flags.sl") rescue nil
    fixture = slurp("tests/builtins/_fixtures/feature_flags_fixture.sl") rescue nil
    skip("not run from the repository root") if template.nil? || fixture.nil?

    assert_eq(fixture, template)
  end
end

describe("FeatureFlags without a backend") do
  test("an unknown flag is off") do
    assert_not(FeatureFlags.enabled?("never_defined_flag_xyz", user: "u_1"))
  end

  test("a blank name is off") do
    assert_not(FeatureFlags.enabled?(""))
  end

  test("bucket is a stable 0..99 hash of the key") do
    assert_eq(FeatureFlags.bucket("checkout:u_42"), 72)
    assert_eq(FeatureFlags.bucket("checkout:u_42"), 72)
    assert_eq(FeatureFlags.bucket("anything"), 83)
    assert_eq(FeatureFlags.bucket(""), 0)
  end

  test("clamp pins a percent into 0..100") do
    assert_eq(FeatureFlags.clamp(-10), 0)
    assert_eq(FeatureFlags.clamp(0), 0)
    assert_eq(FeatureFlags.clamp(37), 37)
    assert_eq(FeatureFlags.clamp(100), 100)
    assert_eq(FeatureFlags.clamp(250), 100)
  end

  test("key namespaces the flag under feature:") do
    assert_eq(FeatureFlags.key("checkout_v2"), "feature:checkout_v2")
  end

  test("the TTL defaults to about ten years") do
    assert_eq(FeatureFlags.ttl, 315360000)
  end

  describe("environment overrides") do
    test("a truthy SOLI_FEATURE_ variable forces the flag on") do
      assert(FeatureFlags.enabled?("ff_test_on", user: "u_1"))
      assert(FeatureFlags.enabled?("ff_test_on"))
      assert_eq(FeatureFlags.env_override("ff_test_on"), true)
    end

    test("a falsy SOLI_FEATURE_ variable forces the flag off") do
      assert_not(FeatureFlags.enabled?("ff_test_off", user: "u_1"))
      assert_eq(FeatureFlags.env_override("ff_test_off"), false)
    end

    test("no variable means no override") do
      assert_null(FeatureFlags.env_override("never_defined_flag_xyz"))
    end
  end
end

describe("FeatureFlags stored in the cache") do
  before_each() do
    requires_solikv()
    FeatureFlags.clear("spec_flag")
  end

  after_each() do
    FeatureFlags.clear("spec_flag")
  end

  test("get is nil for a flag never stored") do
    assert_null(FeatureFlags.get("spec_flag"))
  end

  test("enable stores an on config with no targeting") do
    FeatureFlags.enable("spec_flag")
    assert_eq(FeatureFlags.get("spec_flag"), {"on": true, "rollout": 100, "users": [], "groups": []})
    assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))
    assert(FeatureFlags.enabled?("spec_flag"))
  end

  test("stores the flag with the long TTL") do
    FeatureFlags.enable("spec_flag")
    assert_eq(Cache.ttl("feature:spec_flag"), 315360000)
  end

  test("disable kills an enabled flag") do
    FeatureFlags.enable("spec_flag")
    FeatureFlags.disable("spec_flag")
    assert_eq(FeatureFlags.get("spec_flag")["on"], false)
    assert_not(FeatureFlags.enabled?("spec_flag", user: "u_1"))
  end

  test("the kill-switch beats an allowlisted user") do
    FeatureFlags.enable_for("spec_flag", "u_1")
    FeatureFlags.disable("spec_flag")
    assert_not(FeatureFlags.enabled?("spec_flag", user: "u_1"))
  end

  test("clear forgets the flag") do
    FeatureFlags.enable("spec_flag")
    FeatureFlags.clear("spec_flag")
    assert_null(FeatureFlags.get("spec_flag"))
    assert_not(FeatureFlags.enabled?("spec_flag", user: "u_1"))
  end

  describe("rollout") do
    test("100 is on for everyone, 0 for no one") do
      FeatureFlags.set_rollout("spec_flag", 100)
      assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))
      FeatureFlags.set_rollout("spec_flag", 0)
      assert_not(FeatureFlags.enabled?("spec_flag", user: "u_1"))
    end

    test("set_rollout clamps the percent it stores") do
      FeatureFlags.set_rollout("spec_flag", 250)
      assert_eq(FeatureFlags.get("spec_flag")["rollout"], 100)
      FeatureFlags.set_rollout("spec_flag", -5)
      assert_eq(FeatureFlags.get("spec_flag")["rollout"], 0)
    end

    test("a user is in when their bucket is below the percent") do
      # bucket("spec_flag:u_1") is 29, bucket("spec_flag:u_2") is 30
      FeatureFlags.set_rollout("spec_flag", 30)
      assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))
      assert_not(FeatureFlags.enabled?("spec_flag", user: "u_2"))
    end

    test("ramping up keeps the users already in") do
      FeatureFlags.set_rollout("spec_flag", 30)
      assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))
      FeatureFlags.set_rollout("spec_flag", 31)
      assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))
      assert(FeatureFlags.enabled?("spec_flag", user: "u_2"))
    end

    test("anonymous traffic is never in a partial rollout") do
      FeatureFlags.set_rollout("spec_flag", 99)
      assert_not(FeatureFlags.enabled?("spec_flag"))
    end
  end

  describe("allowlists") do
    test("an allowlisted user wins over a 0% rollout") do
      FeatureFlags.set_rollout("spec_flag", 0)
      FeatureFlags.enable_for("spec_flag", "u_1")
      assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))
      assert_not(FeatureFlags.enabled?("spec_flag", user: "u_2"))
    end

    test("enable_for stores the user once, as a string") do
      FeatureFlags.enable_for("spec_flag", 42)
      FeatureFlags.enable_for("spec_flag", "42")
      assert_eq(FeatureFlags.get("spec_flag")["users"], ["42"])
      assert(FeatureFlags.enabled?("spec_flag", user: 42))
    end

    test("an allowlisted group wins over a 0% rollout") do
      FeatureFlags.set_rollout("spec_flag", 0)
      FeatureFlags.enable_group("spec_flag", "beta")
      FeatureFlags.enable_group("spec_flag", "beta")
      assert_eq(FeatureFlags.get("spec_flag")["groups"], ["beta"])
      assert(FeatureFlags.enabled?("spec_flag", user: "u_9", groups: ["beta"]))
      assert_not(FeatureFlags.enabled?("spec_flag", user: "u_9", groups: ["other"]))
    end
  end
end
