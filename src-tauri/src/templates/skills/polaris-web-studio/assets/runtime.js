/* polaris-web-studio :: runtime.js — tiny scroll-reveal + active-nav
 *
 * Adds `.in` to `.reveal` elements as they enter the viewport (one-shot),
 * and a subtle theme cycler on the `T` key (data-theme on <html>) for previewing.
 */
(function () {
  "use strict";
  var THEMES = [
    "minimal-white", "editorial-serif", "swiss-grid", "magazine-bold",
    "japanese-minimal", "xiaohongshu-white", "academic-paper", "corporate-clean",
    "soft-pastel", "tokyo-night", "dracula", "nord", "cyberpunk-neon",
    "terminal-green", "blueprint", "glassmorphism", "neo-brutalism"
  ];
  function ready(fn) {
    if (document.readyState !== "loading") fn();
    else document.addEventListener("DOMContentLoaded", fn);
  }
  ready(function () {
    var els = Array.prototype.slice.call(document.querySelectorAll(".reveal"));
    if ("IntersectionObserver" in window && els.length) {
      var io = new IntersectionObserver(function (entries) {
        entries.forEach(function (e) {
          if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
        });
      }, { threshold: 0.12, rootMargin: "0px 0px -8% 0px" });
      els.forEach(function (el) { io.observe(el); });
    } else {
      els.forEach(function (el) { el.classList.add("in"); });
    }
    // staggered children: respect inline style --d on each .reveal
    document.addEventListener("keydown", function (e) {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "t" || e.key === "T") {
        var cur = document.documentElement.getAttribute("data-theme") || THEMES[0];
        var i = (THEMES.indexOf(cur) + (e.shiftKey ? -1 : 1) + THEMES.length) % THEMES.length;
        document.documentElement.setAttribute("data-theme", THEMES[i]);
      }
    });
  });
})();
