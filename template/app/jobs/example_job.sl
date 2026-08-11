# Background job run by Soli's in-process job engine. Put your work in
# `static def perform`.
#
# Enqueue:              ExampleJob.perform_later({ "to": "alice@example.com" });
# Run inline (tests):   ExampleJob.perform_now({ "to": "alice@example.com" });
# Recurring:            ExampleJob.schedule_cron("welcome_blast", Cron.daily_at("09:00"), {});
#
# Or declare the schedule on the class (note the type annotation):
#   static cron: String = Cron.daily_at("09:00");

class ExampleJob {
    static def perform(args: Hash) {
        print("ExampleJob ran with: " + str(args));
    }
}
