When an order is created, enqueue `NotifyJob` with the new order id.
Use `NotifyJob.perform_later`. Add a `create` action on `OrdersController`
that reads `params["id"]` (or the created record id) and enqueues.

Do not call `perform_now` as the only path.
