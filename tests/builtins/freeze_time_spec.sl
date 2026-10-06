describe("freeze_time", fn() {
  test("pins datetime_now until unfreeze", fn() {
    freeze_time(1700000000)
    first = datetime_now()
    second = datetime_now()
    assert_eq(first, 1700000000)
    assert_eq(second, 1700000000)

    unfreeze_time()
    unfrozen = datetime_now()
    freeze_time(1700000001)
    assert_eq(datetime_now(), 1700000001)
    unfreeze_time()
    assert(unfrozen != 1700000001 || datetime_now() != 1700000001)
  })

  test("travel_to is an alias", fn() {
    travel_to(1715212800)
    assert_eq(datetime_now(), 1715212800)
    unfreeze_time()
  })
})
