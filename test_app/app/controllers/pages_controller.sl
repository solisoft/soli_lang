// Pages controller

fn index() {
  render("pages/index.html.erb", {
    title: "SoliLang LiveView Demo"
  })
}

fn counter() {
  render("pages/counter.html.erb", {
    title: "LiveView Counter Demo"
  })
}
