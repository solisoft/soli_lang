// Light / dark theme. Loaded synchronously in <head> so the attribute is set
// before first paint. Order: saved choice, then the system preference.
(function () {
  var key = 'soli-theme';
  var root = document.documentElement;
  var theme = null;
  try { theme = localStorage.getItem(key); } catch (e) {}
  if (theme !== 'light' && theme !== 'dark') {
    theme = window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }
  root.setAttribute('data-theme', theme);

  window.toggleTheme = function () {
    var next = root.getAttribute('data-theme') === 'light' ? 'dark' : 'light';
    root.setAttribute('data-theme', next);
    try { localStorage.setItem(key, next); } catch (e) {}
  };
})();
