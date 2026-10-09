# What every page shows about the request that produced it. Not a database
# model: a plain class, kept in app/models so it loads before the controllers.
class RuntimeInfo
  static def of(req)
    headers = req["headers"]
    ray = headers["cf-ray"].to_s
    {
      "runtime": getenv("SOLI_RUNTIME") ?? "soli serve",
      "version": getenv("SOLI_VERSION") ?? "dev",
      "colo": headers["cf-colo"] ?? (ray.includes?("-") ? ray.split("-").last : "local"),
      "country": headers["cf-ipcountry"] ?? "unknown",
      "rendered_at": DateTime.utc.to_iso.substring(0, 19) + "Z"
    }
  end
end
