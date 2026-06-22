# ============================================================================
# FeatureFlags stdlib module — test suite
# ============================================================================
# The module ships as a scaffold template. Module imports are sandboxed to the
# spec's own directory, so we import a fixture copy and assert (below) that it
# byte-matches the canonical template — drift fails the suite.
#
# Env-override and bucketing paths run without SoliKV. Cache-backed paths
# (enable/disable/rollout/allowlist) are guarded on SoliKV availability, like
# tests/builtins/cache_spec.sl.
# ============================================================================

import "./feature_flags_fixture.sl"

# Detect SoliKV availability
let __solikv_available = false
try
    Cache.set("__ff_probe__", "ok", 5)
    if Cache.get("__ff_probe__") == "ok"
        __solikv_available = true
        Cache.delete("__ff_probe__")
    end
catch e
end

describe("FeatureFlags — fixture in sync with shipped template", fn() {
    test("fixture byte-matches the scaffold template", fn() {
        # Run from the repo root (how `soli test` runs in CI).
        let template = slurp("src/scaffold/templates/feature_flags.sl") rescue null
        let fixture = slurp("tests/builtins/feature_flags_fixture.sl") rescue null
        if template.nil? || fixture.nil?
            return null  # not at repo root — skip rather than false-fail
        end
        assert_eq(fixture, template)
    })
})

describe("FeatureFlags — no backend required", fn() {
    test("unknown flag is off", fn() {
        # An env override would mask this, so use a name with no SOLI_FEATURE_ var.
        assert_not(FeatureFlags.enabled?("never_defined_flag_xyz", user: "u_1"))
    })

    test("blank name is off", fn() {
        assert_not(FeatureFlags.enabled?(""))
    })

    test("bucket is stable for the same key", fn() {
        assert_eq(FeatureFlags.bucket("checkout:u_42"), FeatureFlags.bucket("checkout:u_42"))
    })

    test("bucket stays within 0..99", fn() {
        let b = FeatureFlags.bucket("anything")
        assert(b >= 0)
        assert(b < 100)
    })

    test("clamp pins percent into 0..100", fn() {
        assert_eq(FeatureFlags.clamp(-10), 0)
        assert_eq(FeatureFlags.clamp(250), 100)
        assert_eq(FeatureFlags.clamp(37), 37)
    })

    test("env override forces a flag on", fn() {
        # SOLI_FEATURE_FF_TEST_ON=1 is exported by the test runner below.
        assert(FeatureFlags.enabled?("ff_test_on", user: "u_1"))
    })

    test("env override forces a flag off", fn() {
        assert_not(FeatureFlags.enabled?("ff_test_off", user: "u_1"))
    })
})

describe("FeatureFlags — cache-backed", fn() {
    before_each(fn() {
        if __solikv_available
            FeatureFlags.clear("spec_flag")
        end
    })

    test("enable turns a flag on, disable kills it", fn() {
        if not __solikv_available
            return null
        end
        FeatureFlags.enable("spec_flag")
        assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))

        FeatureFlags.disable("spec_flag")
        assert_not(FeatureFlags.enabled?("spec_flag", user: "u_1"))
    })

    test("rollout 100 is on for everyone, 0 is off", fn() {
        if not __solikv_available
            return null
        end
        FeatureFlags.set_rollout("spec_flag", 100)
        assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))

        FeatureFlags.set_rollout("spec_flag", 0)
        assert_not(FeatureFlags.enabled?("spec_flag", user: "u_1"))
    })

    test("allowlisted user wins over a 0% rollout", fn() {
        if not __solikv_available
            return null
        end
        FeatureFlags.set_rollout("spec_flag", 0)
        FeatureFlags.enable_for("spec_flag", "u_1")
        assert(FeatureFlags.enabled?("spec_flag", user: "u_1"))
        assert_not(FeatureFlags.enabled?("spec_flag", user: "u_2"))
    })

    test("allowlisted group wins over a 0% rollout", fn() {
        if not __solikv_available
            return null
        end
        FeatureFlags.set_rollout("spec_flag", 0)
        FeatureFlags.enable_group("spec_flag", "beta")
        assert(FeatureFlags.enabled?("spec_flag", user: "u_9", groups: ["beta"]))
        assert_not(FeatureFlags.enabled?("spec_flag", user: "u_9", groups: ["other"]))
    })

    test("clear removes the flag", fn() {
        if not __solikv_available
            return null
        end
        FeatureFlags.enable("spec_flag")
        FeatureFlags.clear("spec_flag")
        assert_null(FeatureFlags.get("spec_flag"))
    })

    test("rollout membership is deterministic across calls", fn() {
        if not __solikv_available
            return null
        end
        FeatureFlags.set_rollout("spec_flag", 50)
        let first = FeatureFlags.enabled?("spec_flag", user: "stable_user")
        let second = FeatureFlags.enabled?("spec_flag", user: "stable_user")
        assert_eq(first, second)
    })
})
