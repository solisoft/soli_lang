class NotifyJob {
    static def perform(args: Hash) {
        print("NotifyJob: " + str(args))
    }
}
