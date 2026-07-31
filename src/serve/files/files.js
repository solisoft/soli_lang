// Client behaviour for file mode: theme toggle, tree filter, keyboard nav.
//
// The sidebar is rendered server-side and every row is a real link, so this
// script only narrows what is already there. With JS off the tree still works.
(function () {
  "use strict";

  var root = document.documentElement;
  var THEME_KEY = "soli-files-theme";

  // ---- theme -------------------------------------------------------------

  var themeButton = document.getElementById("theme");
  if (themeButton) {
    themeButton.addEventListener("click", function () {
      var current = root.dataset.theme;
      if (!current) {
        // No explicit choice yet: flip away from whatever the OS is showing.
        var prefersLight = window.matchMedia("(prefers-color-scheme: light)").matches;
        current = prefersLight ? "light" : "dark";
      }
      var next = current === "dark" ? "light" : "dark";
      root.dataset.theme = next;
      try {
        localStorage.setItem(THEME_KEY, next);
      } catch (e) {
        /* private mode — the toggle still works for this page */
      }
    });
  }

  // ---- off-canvas sidebar on narrow screens ------------------------------

  var side = document.getElementById("side");
  var menuButton = document.getElementById("menu");
  if (side && menuButton) {
    menuButton.addEventListener("click", function () {
      var open = side.classList.toggle("open");
      menuButton.setAttribute("aria-expanded", open ? "true" : "false");
    });
  }

  // ---- bring the current file into view ----------------------------------

  // The sidebar scrolls independently of the page, so on a deep tree the row
  // for the file you are reading can sit hundreds of rows below the fold —
  // the rail is lit, and you cannot see it. Centre it on load.
  var currentRow = document.querySelector(".tree .row.cur");
  if (currentRow && side) {
    var rowBox = currentRow.getBoundingClientRect();
    var sideBox = side.getBoundingClientRect();
    var above = rowBox.top < sideBox.top;
    var below = rowBox.bottom > sideBox.bottom;
    // Leave a short tree alone: jumping an already-visible row is just motion.
    if (above || below) {
      side.scrollTop +=
        rowBox.top - sideBox.top - side.clientHeight / 2 + rowBox.height / 2;
    }
  }

  // ---- outline: mark the section you are reading -------------------------

  var tocLinks = Array.prototype.slice.call(document.querySelectorAll(".toc a"));
  if (tocLinks.length && "IntersectionObserver" in window) {
    var byId = Object.create(null);
    var targets = [];
    tocLinks.forEach(function (link) {
      var heading = document.getElementById(decodeURIComponent(link.hash.slice(1)));
      if (heading) {
        byId[heading.id] = link;
        targets.push(heading);
      }
    });

    // Track which headings are on screen rather than reacting to whichever
    // one last crossed the line — scrolling up past a short section would
    // otherwise leave the mark behind.
    var visible = Object.create(null);
    var observer = new IntersectionObserver(
      function (records) {
        records.forEach(function (record) {
          if (record.isIntersecting) {
            visible[record.target.id] = true;
          } else {
            delete visible[record.target.id];
          }
        });

        var current = null;
        for (var i = 0; i < targets.length; i++) {
          if (visible[targets[i].id]) {
            current = targets[i].id;
            break;
          }
        }
        // Nothing visible means we are inside a long section: keep the last
        // heading we scrolled past marked.
        if (!current) {
          for (var j = targets.length - 1; j >= 0; j--) {
            if (targets[j].getBoundingClientRect().top < 80) {
              current = targets[j].id;
              break;
            }
          }
        }

        tocLinks.forEach(function (link) {
          link.classList.remove("on");
        });
        if (current && byId[current]) {
          byId[current].classList.add("on");
        }
      },
      // Ignore the strip under the sticky bar so a heading counts as "here"
      // only once it is genuinely in the reading area.
      { rootMargin: "-60px 0px -70% 0px", threshold: 0 }
    );

    targets.forEach(function (heading) {
      observer.observe(heading);
    });
  }

  // ---- filter ------------------------------------------------------------

  var input = document.getElementById("q");
  var rows = Array.prototype.slice.call(document.querySelectorAll(".tree .row"));
  if (!input || !rows.length) {
    return;
  }

  // The filter key is the row's path, derived from its href rather than
  // duplicated into an attribute — on a large tree that attribute doubled the
  // weight of the sidebar. Computed once here, then cached on the element.
  rows.forEach(function (row) {
    var href = row.getAttribute("href") || "";
    try {
      href = decodeURIComponent(href);
    } catch (e) {
      /* malformed escape — fall back to the raw href */
    }
    row.__p = href.replace(/^\//, "").replace(/\/$/, "").toLowerCase();
  });

  var cursor = -1;

  function visible() {
    return rows.filter(function (row) {
      return !row.classList.contains("hide");
    });
  }

  function clearCursor() {
    rows.forEach(function (row) {
      row.classList.remove("k");
    });
    cursor = -1;
  }

  function apply(query) {
    clearCursor();
    var needle = query.trim().toLowerCase();

    if (!needle) {
      rows.forEach(function (row) {
        row.classList.remove("hide");
      });
      return;
    }

    var keep = Object.create(null);
    rows.forEach(function (row) {
      var path = row.__p;
      if (path.indexOf(needle) === -1) {
        return;
      }
      keep[path] = true;
      // Keep the chain of folders leading to the match, otherwise a nested
      // hit appears to float with no context.
      var cut = path.lastIndexOf("/");
      while (cut > -1) {
        path = path.slice(0, cut);
        keep[path] = true;
        cut = path.lastIndexOf("/");
      }
    });

    rows.forEach(function (row) {
      row.classList.toggle("hide", !keep[row.__p]);
    });
  }

  function move(delta) {
    var list = visible();
    if (!list.length) {
      return;
    }
    if (cursor > -1 && list[cursor]) {
      list[cursor].classList.remove("k");
    }
    cursor += delta;
    if (cursor < 0) {
      cursor = list.length - 1;
    }
    if (cursor >= list.length) {
      cursor = 0;
    }
    var row = list[cursor];
    row.classList.add("k");
    row.scrollIntoView({ block: "nearest" });
  }

  input.addEventListener("input", function () {
    apply(input.value);
  });

  input.addEventListener("keydown", function (event) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      move(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      move(-1);
    } else if (event.key === "Enter") {
      var list = visible();
      var target = cursor > -1 ? list[cursor] : list[0];
      if (target) {
        event.preventDefault();
        window.location.href = target.href;
      }
    } else if (event.key === "Escape") {
      input.value = "";
      apply("");
      input.blur();
    }
  });

  // `/` focuses the filter from anywhere on the page.
  document.addEventListener("keydown", function (event) {
    if (event.key !== "/" || event.metaKey || event.ctrlKey || event.altKey) {
      return;
    }
    var active = document.activeElement;
    var tag = active && active.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || (active && active.isContentEditable)) {
      return;
    }
    event.preventDefault();
    input.focus();
    input.select();
  });
})();
