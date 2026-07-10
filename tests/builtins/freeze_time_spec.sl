describe("freeze_time", fn() {
    test("pins datetime_now until unfreeze", fn() {
        freeze_time(1_700_000_000)
        first = datetime_now()
        second = datetime_now()
        assert_eq(first, 1_700_000_000)
        assert_eq(second, 1_700_000_000)

        unfreeze_time()
        unfrozen = datetime_now()
        freeze_time(1_700_000_001)
        assert_eq(datetime_now(), 1_700_000_001)
        unfreeze_time()
        assert(unfrozen != 1_700_000_001 || datetime_now() != 1_700_000_001)
    })

    test("travel_to is an alias", fn() {
        travel_to(1_715_212_800)
        assert_eq(datetime_now(), 1_715_212_800)
        unfreeze_time()
    })
})