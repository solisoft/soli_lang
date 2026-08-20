In `OrdersController#index`, load orders whose `total` is greater than 10 using
the portable hash filter form of `Order.where`. Return them as JSON under
the key `"orders"`.

Do not write raw SDBQL (`"doc.total > @n"`).
