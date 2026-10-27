// The registry pages work without scripts; this adds the theme toggle, the
// phone menu, copy buttons, filters that apply when changed, the version
// picker and the reserved names' filter and edit buttons.
(function () {
  "use strict";

  function setTheme(choice) {
    var root = document.documentElement;
    if (choice === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", choice);
    try {
      if (choice === "system") localStorage.removeItem("wl-theme");
      else localStorage.setItem("wl-theme", choice);
    } catch (e) {}
    showTheme();
  }

  function storedTheme() {
    try {
      var t = localStorage.getItem("wl-theme");
      return t === "dark" || t === "light" ? t : "system";
    } catch (e) {
      return "system";
    }
  }

  function showTheme() {
    var current = storedTheme();
    document.querySelectorAll("[data-theme-choice]").forEach(function (b) {
      b.setAttribute("aria-checked", String(b.getAttribute("data-theme-choice") === current));
    });
  }

  function copyText(button) {
    var text = button.getAttribute("data-copy");
    if (!text) {
      var box = button.closest(".wl-code, .wl-copy-cmd, .wl-pkghead__install, .wl-term");
      var source = box && box.querySelector("pre, code, .wl-copy-cmd__text");
      text = source ? source.textContent.replace(/^\$ /, "") : "";
    }
    if (!navigator.clipboard) return;
    navigator.clipboard.writeText(text.replace(/\s+$/, "")).then(function () {
      var label = button.textContent;
      button.setAttribute("data-state", "copied");
      button.textContent = "copied";
      setTimeout(function () {
        button.removeAttribute("data-state");
        button.textContent = label;
      }, 1500);
    });
  }

  function menu(open) {
    var sheet = document.getElementById("rg-menu");
    if (!sheet) return;
    sheet.hidden = !open;
    document.body.style.overflow = open ? "hidden" : "";
    var opener = document.querySelector("[data-menu-open]");
    if (opener) opener.setAttribute("aria-expanded", String(open));
    var focus = open ? sheet.querySelector("[data-menu-close]") : opener;
    if (focus) focus.focus();
  }

  function filterRows(input) {
    var table = document.getElementById(input.getAttribute("data-filter-rows"));
    var count = document.getElementById(input.getAttribute("data-filter-count"));
    if (!table) return;
    var words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
    var rows = table.querySelectorAll("tbody tr");
    var shown = 0;
    rows.forEach(function (row) {
      var text = row.textContent.toLowerCase();
      var match = words.every(function (w) { return text.indexOf(w) >= 0; });
      row.hidden = !match;
      if (match) shown++;
    });
    if (count) count.textContent = shown + " of " + rows.length + " names";
  }

  document.addEventListener("click", function (e) {
    var t = e.target.closest("[data-theme-choice], [data-copy], .wl-textbtn, .wl-copy-cmd__btn, [data-menu-open], [data-menu-close], [data-edit]");
    if (!t) return;
    if (t.hasAttribute("data-theme-choice")) setTheme(t.getAttribute("data-theme-choice"));
    else if (t.hasAttribute("data-menu-open")) menu(true);
    else if (t.hasAttribute("data-menu-close")) menu(false);
    else if (t.hasAttribute("data-edit")) {
      var form = document.getElementById(t.getAttribute("data-edit"));
      if (!form) return;
      ["name", "note", "authors"].forEach(function (k) {
        var field = form.elements[k];
        if (field) field.value = t.getAttribute("data-" + k) || "";
      });
      form.scrollIntoView({ block: "center" });
      if (form.elements.authors) form.elements.authors.focus();
    } else copyText(t);
  });

  document.addEventListener("keydown", function (e) {
    var sheet = document.getElementById("rg-menu");
    if (e.key === "Escape" && sheet && !sheet.hidden) menu(false);
  });

  document.addEventListener("change", function (e) {
    var t = e.target;
    if (t.hasAttribute("data-autosubmit") && t.form) t.form.submit();
    else if (t.hasAttribute("data-navigate") && t.value) location.href = t.value;
    else if (t.hasAttribute("data-kind-picker")) {
      var form = document.getElementById(t.getAttribute("data-kind-picker"));
      if (!form) return;
      Array.prototype.forEach.call(form.elements, function (box) {
        if (box.name !== "kind") return;
        box.checked = t.value === "" || box.value === t.value;
      });
      form.submit();
    }
  });

  document.addEventListener("input", function (e) {
    if (e.target.hasAttribute("data-filter-rows")) filterRows(e.target);
  });

  showTheme();
  document.querySelectorAll("[data-filter-rows]").forEach(filterRows);
})();
