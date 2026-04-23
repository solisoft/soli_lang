# Global middleware that stamps every request with a marker on the request
# object. A downstream action can echo it back so an e2e test can prove the
# middleware ran without needing to scrape response headers (Soli middleware
# response-header injection works differently across code paths; request-side
# mutation is the portable signal).

# order: 10

fn tag_request(req: Any)
    req["middleware_stamp"] = "middleware_saw_request";
    return {
        "continue": true,
        "request": req
    };
end
