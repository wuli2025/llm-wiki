/* polaris-deck-studio :: runtime.js — minimal, dependency-free deck engine
 *
 * Clean-room Polaris implementation (not vendored). Wires up a `.deck > .slide` deck:
 *   ← / → / Space / PgUp / PgDn / Home / End  navigate
 *   T  cycle theme (data-theme on <html>)      F  fullscreen
 *   O  overview grid (click a thumb to jump)   P  print (→ PDF)
 *   #/N deep-link to slide N (1-based)         (used by export-pptx.mjs)
 *
 * Export hooks (for headless screenshotting):
 *   window.__deck = { total, current(), go(n), next(), prev() }
 *   add ?export=1 to the URL (or <html class="no-anim">) to disable entrance animations.
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
    var deck = document.querySelector(".deck");
    var slides = Array.prototype.slice.call(document.querySelectorAll(".slide"));
    if (!slides.length) return;
    var total = slides.length;
    var idx = 0;

    // Export mode: kill entrance animations for clean, deterministic stills.
    if (/[?&]export=1/.test(location.search)) {
      document.documentElement.classList.add("no-anim");
    }

    // progress bar
    var prog = document.querySelector(".progress-bar > span");
    // slide-number chrome (any element with .slide-number gets data-current/total)
    var counters = Array.prototype.slice.call(document.querySelectorAll(".slide-number"));

    function clamp(n) { return Math.max(0, Math.min(total - 1, n)); }

    function render() {
      for (var i = 0; i < total; i++) {
        var s = slides[i];
        s.classList.toggle("is-active", i === idx);
        s.classList.toggle("is-prev", i < idx);
      }
      if (prog) prog.style.width = ((idx + 1) / total * 100) + "%";
      for (var c = 0; c < counters.length; c++) {
        counters[c].setAttribute("data-current", String(idx + 1));
        counters[c].setAttribute("data-total", String(total));
      }
      var hash = "#/" + (idx + 1);
      if (location.hash !== hash) {
        try { history.replaceState(null, "", hash); } catch (e) { location.hash = hash; }
      }
      document.title = document.title.replace(/\s+·\s+\d+\/\d+$/, "");
    }

    function go(n) { idx = clamp(n); render(); }
    function next() { if (idx < total - 1) go(idx + 1); }
    function prev() { if (idx > 0) go(idx - 1); }

    // ---- deep link from hash (#/3) ----
    function fromHash() {
      var m = /^#\/(\d+)/.exec(location.hash || "");
      if (m) { var n = parseInt(m[1], 10) - 1; if (!isNaN(n)) idx = clamp(n); }
    }
    fromHash();
    window.addEventListener("hashchange", function () {
      var m = /^#\/(\d+)/.exec(location.hash || "");
      if (m) { var n = parseInt(m[1], 10) - 1; if (!isNaN(n) && n !== idx) go(n); }
    });

    // ---- theme cycling ----
    function cycleTheme(dir) {
      var cur = document.documentElement.getAttribute("data-theme") || THEMES[0];
      var i = THEMES.indexOf(cur);
      i = (i + (dir || 1) + THEMES.length) % THEMES.length;
      document.documentElement.setAttribute("data-theme", THEMES[i]);
    }

    // ---- overview grid ----
    var overview = document.querySelector(".overview");
    function buildOverview() {
      if (!overview || overview.dataset.built) return;
      for (var i = 0; i < total; i++) {
        var t = document.createElement("div");
        t.className = "thumb";
        var title = slides[i].getAttribute("data-title") ||
          (slides[i].querySelector("h1,h2,.h1,.h2,h3") || {}).textContent || ("Slide " + (i + 1));
        t.innerHTML = '<span class="n">' + (i + 1) + '</span><span class="t"></span>';
        t.querySelector(".t").textContent = String(title).trim().slice(0, 60);
        (function (n) { t.addEventListener("click", function () { go(n); toggleOverview(false); }); })(i);
        overview.appendChild(t);
      }
      overview.dataset.built = "1";
    }
    function toggleOverview(force) {
      if (!overview) return;
      buildOverview();
      var open = force === undefined ? !overview.classList.contains("open") : force;
      overview.classList.toggle("open", open);
    }

    function toggleFullscreen() {
      if (!document.fullscreenElement) (document.documentElement.requestFullscreen || function () {}).call(document.documentElement);
      else (document.exitFullscreen || function () {}).call(document);
    }

    // ---- keyboard ----
    document.addEventListener("keydown", function (e) {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      switch (e.key) {
        case "ArrowRight": case "PageDown": case " ": case "Spacebar":
          e.preventDefault(); next(); break;
        case "ArrowLeft": case "PageUp":
          e.preventDefault(); prev(); break;
        case "Home": e.preventDefault(); go(0); break;
        case "End": e.preventDefault(); go(total - 1); break;
        case "t": case "T": cycleTheme(e.shiftKey ? -1 : 1); break;
        case "f": case "F": toggleFullscreen(); break;
        case "o": case "O": case "Escape": toggleOverview(e.key === "Escape" ? false : undefined); break;
        case "p": case "P": window.print(); break;
        default: break;
      }
    });

    // click navigation (right half = next, left quarter = prev)
    if (deck) {
      deck.addEventListener("click", function (e) {
        if (e.target.closest("a,button,input,textarea,.no-nav,.overview")) return;
        var x = e.clientX / window.innerWidth;
        if (x > 0.6) next(); else if (x < 0.25) prev();
      });
    }

    render();
    window.__deck = { total: total, current: function () { return idx; }, go: go, next: next, prev: prev };
  });
})();
