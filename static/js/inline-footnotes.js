// discourse-footnote's inline footnotes (api-initializers/inline-footnotes.gjs),
// which the layout loads when display_footnotes_inline is on: each
// sup.footnote-ref in a post becomes span.inline-footnote holding a
// DTooltip (identifier "inline-footnote", interactive, closed by a click
// outside but not by scrolling) whose trigger is a.expand-footnote and
// whose content is the footnote without its back references; the post
// gets .inline-footnotes, whose CSS numbers the links and hides the list.
(function () {
  "use strict";

  if (!Discourse.once("inline-footnotes")) {
    return;
  }

  Discourse.decorateCooked("inline-footnotes", function (elem) {
    var footnoteRefs = elem.querySelectorAll("sup.footnote-ref");

    footnoteRefs.forEach(function (footnoteRef) {
      var refLink = footnoteRef.querySelector("a");
      if (!refLink) {
        return;
      }

      var footnoteId = refLink.getAttribute("href");
      var found = elem.querySelector(footnoteId);
      var footnoteElement = found ? found.cloneNode(true) : null;

      if (footnoteElement) {
        footnoteElement.querySelectorAll("sup.footnote-ref, .footnote-backref").forEach(function (element) {
          element.remove();
        });
      }

      var footnoteContent = footnoteElement ? footnoteElement.innerHTML : undefined;

      var expandableFootnote = document.createElement("span");
      expandableFootnote.className = "inline-footnote";
      footnoteRef.replaceWith(expandableFootnote);

      // InlineFootnote: the trigger link, which only opens the tooltip.
      var link = document.createElement("a");
      link.className = "expand-footnote";
      if (footnoteContent !== undefined) {
        link.dataset.footnoteContent = footnoteContent;
      }
      link.dataset.footnoteId = footnoteId;
      link.setAttribute("href", "");
      link.setAttribute("role", "button");
      link.addEventListener("click", function (event) {
        event.preventDefault();
      });

      var tooltip = Discourse.float.tooltip(link, {
        closeOnClickOutside: true,
        closeOnScroll: false,
        identifier: "inline-footnote",
        interactive: true,
        content: footnoteContent === undefined ? null : footnoteContent,
      });
      expandableFootnote.appendChild(tooltip.trigger);
    });

    if (footnoteRefs.length) {
      elem.classList.add("inline-footnotes");
    }
  });
})();
