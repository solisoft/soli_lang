In `OrdersController#create`, whitelist incoming params with `permit` so only
`email` and `total` can be mass-assigned. Pass the permitted hash to
`Order.create`.
