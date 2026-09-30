# Navigation data for the shared layout chrome (top bar + footer).

def nav_links
    return [
        { "label": "Dashboard", "href": "/" },
        { "label": "JSON",      "href": "/bench/json" },
        { "label": "Page demo", "href": "/bench/page" },
        { "label": "Health",    "href": "/health" }
    ]
end

def footer_links
    return [
        { "label": "Dashboard", "href": "/" },
        { "label": "Report",    "href": "/bench/page" },
        { "label": "Health",    "href": "/health" }
    ]
end
