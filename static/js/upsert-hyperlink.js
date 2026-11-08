// components/modal/upsert-hyperlink.gjs over Discourse.modal (modal.js),
// shipped in front of composer.js: the link modal of the toolbar's link
// button (and mod+K) and of the rich editor's link toolbar's Edit. Its
// url field searches the forum's topics (searchForTerm with
// type_filter=topic, 400ms after typing, from 4 characters that do not
// start with "http"); the arrow keys and Enter pick a result.
//
//   Discourse.upsertHyperlink.show({ hasSelection, editing, linkText,
//     linkUrl, toolbarEvent }, { i18n, basePath })
//       wraps the selection in the link (toolbarEvent.applyLink) when there
//       is one and nothing is being edited, else adds [text](url)
//       (toolbarEvent.addText).
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.upsertHyperlink) {
    return;
  }

  function escapeHtml(s) {
    return String(s === null || s === undefined ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // lib/url's prefixProtocol
  function prefixProtocol(url) {
    if (!url || typeof url !== "string") {
      return url;
    }
    url = url.trim();
    if (url.startsWith("//")) {
      return "https:" + url;
    }
    if (url.startsWith("/") || url.indexOf("://") !== -1 || url.startsWith("mailto:") || url.startsWith("#")) {
      return url;
    }
    return "https://" + url;
  }

  // onFormSubmit's url: prefixed, parsed (loose % characters encoded
  // first), without the slash URL adds to a bare origin.
  function normalizeUrl(origLink) {
    var linkUrl = prefixProtocol(origLink);
    if (typeof linkUrl === "string") {
      try {
        var safeLinkUrl = linkUrl.replace(/%(?![0-9A-Fa-f]{2})/g, "%25");
        var parsed = new URL(safeLinkUrl, window.location.origin);
        linkUrl = parsed.toString();
        if (linkUrl.endsWith("/") && !safeLinkUrl.endsWith("/") && parsed.pathname === "/") {
          linkUrl = linkUrl.slice(0, -1);
        }
      } catch (e) {
        linkUrl = encodeURI(linkUrl);
      }
    }
    return linkUrl;
  }

  var ids = 0;

  function field(name, title, value, className, placeholder, required) {
    var id = "upsert-hyperlink-" + name + "-" + ++ids;
    return (
      '<div class="form-kit__container form-kit__field form-kit__field-input' +
      (required ? " --required" : "") +
      '" data-name="' + name + '"><label class="form-kit__container-title" for="' + id + '"><span>' +
      escapeHtml(title) +
      '</span></label><div class="form-kit__container-content --full"><div class="form-kit__input-wrapper">' +
      '<input type="text" class="form-kit__control-input ' + className + '" id="' + id + '" name="' + name +
      '" value="' + escapeHtml(value) + '" placeholder="' + escapeHtml(placeholder) + '" autocomplete="off"' + (required ? " autofocus" : "") + ">" +
      '</div><p class="form-kit__errors" hidden></p></div></div>'
    );
  }

  function show(model, opts) {
    if (!Discourse.modal) {
      return null;
    }
    var t = opts.i18n;
    var base = opts.basePath || "";
    var wrapsSelection = model.hasSelection && !model.editing;
    var selectedRow = -1;
    var results = [];
    var timer = null;
    var active = null;
    var close;

    var inputs = document.createElement("div");
    inputs.className = "inputs";
    var form = document.createElement("form");
    form.className = "form-kit";
    form.noValidate = true;
    form.innerHTML =
      field("linkUrl", t("composer.link_url_label"), model.linkUrl || "", "link-url", t("composer.link_url_placeholder"), true) +
      '<div class="internal-link-results" hidden></div>' +
      (wrapsSelection
        ? ""
        : field("linkText", t("composer.link_text_label"), model.linkText || "", "link-text", t("composer.link_optional_text")));
    inputs.appendChild(form);
    var url = form.querySelector("input.link-url");
    var resultsEl = form.querySelector(".internal-link-results");

    function abortSearch() {
      clearTimeout(timer);
      if (active) {
        active.abort();
        active = null;
      }
      setResults([]);
      inputs.querySelectorAll(".spinner").forEach(function (el) {
        el.remove();
      });
    }

    function setResults(list) {
      results = list;
      selectedRow = -1;
      resultsEl.hidden = !list.length;
      resultsEl.innerHTML = list
        .map(function (topic) {
          return (
            '<a class="search-link" data-title="' + escapeHtml(topic.fancy_title) + '" href="' +
            escapeHtml(base + "/t/" + topic.slug + "/" + topic.id) + '">' +
            '<span class="topic-title">' + escapeHtml(topic.title) + "</span></a>"
          );
        })
        .join("");
    }

    // triggerSearch
    function search(value) {
      clearTimeout(timer);
      timer = setTimeout(function () {
        if (!value || value.length < 4 || value.startsWith("http")) {
          abortSearch();
          return;
        }
        if (active) {
          active.abort();
        }
        var controller = new AbortController();
        active = controller;
        var spinner = document.createElement("div");
        spinner.className = "spinner";
        inputs.appendChild(spinner);
        fetch(base + "/search/query.json?" + new URLSearchParams({ term: value, type_filter: "topic" }), {
          credentials: "same-origin",
          headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
          signal: controller.signal,
        })
          .then(function (r) {
            return r.ok ? r.json() : { topics: [] };
          })
          .then(function (json) {
            if (active === controller) {
              setResults(json.topics || []);
            }
          })
          .catch(function () {})
          .finally(function () {
            if (active === controller) {
              active = null;
            }
            spinner.remove();
          });
      }, 400);
    }

    function selectLink(el) {
      setResults([]);
      url.value = el.href;
      var text = form.querySelector("input.link-text");
      if (text) {
        text.value = el.dataset.title;
        text.focus();
      }
    }

    function highlightRow(event, direction) {
      var index = direction === "down" ? selectedRow + 1 : selectedRow - 1;
      var links = resultsEl.querySelectorAll(".search-link");
      if (index > -1 && index < links.length) {
        links[index].focus();
        selectedRow = index;
      } else {
        selectedRow = -1;
        url.focus();
      }
      event.preventDefault();
    }

    // @validation="required" on the url
    function validate() {
      var el = form.querySelector('[data-name="linkUrl"]');
      var errors = el.querySelector(".form-kit__errors");
      var missing = url.value === "";
      el.classList.toggle("has-error", missing);
      errors.hidden = !missing;
      errors.textContent = missing ? t("form_kit.errors.required") : "";
      return !missing;
    }

    function submit() {
      if (!validate()) {
        return;
      }
      var origLink = url.value;
      var linkUrl = normalizeUrl(origLink);
      if (!linkUrl) {
        return;
      }
      abortSearch();
      if (wrapsSelection) {
        model.toolbarEvent.applyLink(linkUrl);
      } else {
        var text = form.querySelector("input.link-text");
        var sel = model.toolbarEvent.selected;
        var linkText = (text && text.value) || (sel && sel.value) || origLink || "";
        model.toolbarEvent.addText("[" + linkText.trim() + "](" + linkUrl + ")");
      }
      close();
    }

    url.addEventListener("input", function () {
      search(url.value);
    });
    resultsEl.addEventListener("click", function (event) {
      var link = event.target.closest(".search-link");
      if (link && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        event.stopPropagation();
        selectLink(link);
      }
    });
    form.addEventListener("submit", function (event) {
      event.preventDefault();
      submit();
    });

    close = Discourse.modal.show({
      className: "upsert-hyperlink-modal",
      title: t(model.editing ? "composer.link_edit_title" : "composer.link_dialog_title"),
      body: function (el) {
        el.classList.add("insert-link");
        el.appendChild(inputs);
        var modal = el.closest(".d-modal") || el;
        modal.addEventListener("keydown", function (event) {
          switch (event.key) {
            case "ArrowDown":
              highlightRow(event, "down");
              break;
            case "ArrowUp":
              highlightRow(event, "up");
              break;
            case "Enter":
              // a selected row is picked, then the form submitted
              if (selectedRow > -1) {
                selectLink(resultsEl.querySelectorAll(".search-link")[selectedRow]);
              }
              if (event.target.tagName === "INPUT") {
                submit();
              }
              event.preventDefault();
              event.stopPropagation();
              break;
          }
        });
        modal.addEventListener("mousedown", function (event) {
          if (!event.target.closest(".inputs")) {
            setResults([]);
          }
        });
      },
      footer: function (el) {
        el.innerHTML =
          '<button class="btn btn-text btn-primary" type="submit"><span class="d-button-label">' +
          escapeHtml(t(model.editing ? "composer.link_edit_action" : "composer.link_dialog_action")) +
          "</span></button>" +
          '<button class="btn btn-text btn-transparent" type="button"><span class="d-button-label">' +
          escapeHtml(t("composer.cancel")) +
          "</span></button>";
        el.querySelector(".btn-primary").addEventListener("click", submit);
        el.querySelector(".btn-transparent").addEventListener("click", function () {
          abortSearch();
          close();
        });
      },
      // Esc closes the results first.
      beforeClose: function () {
        if (results.length) {
          setResults([]);
          return false;
        }
      },
      onClose: function () {
        abortSearch();
        var editor = document.querySelector("#reply-control .d-editor-input:not([hidden])");
        if (opts.onClose) {
          opts.onClose();
        } else if (editor) {
          editor.focus();
        }
      },
    });
    return close;
  }

  Discourse.upsertHyperlink = { show: show, normalizeUrl: normalizeUrl };
})();
