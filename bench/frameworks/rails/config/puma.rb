workers Integer(ENV.fetch("WEB_CONCURRENCY", 16))
threads_count = Integer(ENV.fetch("RAILS_MAX_THREADS", 5))
threads threads_count, threads_count
preload_app!
port Integer(ENV.fetch("PORT", 5096))
environment ENV.fetch("RAILS_ENV", "production")
