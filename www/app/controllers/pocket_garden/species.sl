# Everything the game knows about a plant. The window's workers load
# definitions, not variables: a function is one (so is a `const`), a
# top-level variable is not.
export def species
  {
    "radish":     {"name": "Radish",     "price": 2,  "grows": 6,  "sells": 5},
    "sunflower":  {"name": "Sunflower",  "price": 5,  "grows": 12, "sells": 14},
    "strawberry": {"name": "Strawberry", "price": 8,  "grows": 18, "sells": 24},
    "lavender":   {"name": "Lavender",   "price": 12, "grows": 26, "sells": 40}
  }
end

export def species_order
  ["radish", "sunflower", "strawberry", "lavender"]
end
