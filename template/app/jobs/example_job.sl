// Background job triggered by SolidB. Define your handler in `static def perform`.
//
// Enqueue with `ExampleJob.perform_later({ "to": "alice@example.com" });`
// Schedule recurring with `ExampleJob.schedule_cron("welcome_blast", Cron.daily_at("09:00"), {});`

class ExampleJob {
    static def perform(args: Hash) {
        print("ExampleJob ran with: " + str(args));
    }
}
