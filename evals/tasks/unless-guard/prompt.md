In `OrdersController#index`, skip work when there is no `params["q"]` using
a block `unless … end` (not only postfix). Return `{ "orders": [] }` in that
guard.
