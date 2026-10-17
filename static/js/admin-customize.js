// The customize admin pages in the browser (src/admin_customize_view.rs
// draws them). The watched words: the filter (the route's query param),
// the action nav's menu in the mobile view, the regular expression
// errors, WatchedWordForm (the creatable WatchedWords and TagChooser
// MultiSelects, the checks, the save), the show words checkbox, a word's
// delete, clear all, the upload and the test modal. What the server draws
// again (the nav, the list) is fetched as the page and swapped in.
(function () {
  "use strict";

  if (!Discourse.once("admin-customize")) {
    return;
  }

  function ui() {
    return Discourse.adminUi;
  }

  function t(key, args) {
    return ui().t(key, args);
  }

  function escapeHtml(s) {
    return ui().escapeHtml(s);
  }

  // A form's submit, ahead of the boosted navigation's listener (capture),
  // which the handler stops for the forms it takes.
  function onSubmit(fn) {
    document.addEventListener("submit", fn, true);
  }

  function fetchPage(path) {
    return fetch(ui().base() + path, { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        return new DOMParser().parseFromString(html, "text/html");
      });
  }

  // --- select-kit choosers --------------------------------------------------
  //
  // details.multi-select[data-chooser] (src/admin_customize_kit.rs chooser):
  // the selection in data-chosen; data-creatable offers the typed text
  // (WatchedWords, TagChooser's allowAny), data-search the tags' or the
  // users' results, data-maximum stops at that many. `change` fires on
  // the details. details[data-category-chooser]: the CategoryChooser
  // ComboBox over #customize-categories, its value in data-value.

  function chosenOf(details) {
    return JSON.parse(details.dataset.chosen || "[]");
  }

  function full(details) {
    var max = parseInt(details.dataset.maximum, 10);
    return max > 0 && chosenOf(details).length >= max;
  }

  function setChosen(details, chosen) {
    details.dataset.chosen = JSON.stringify(chosen);
    var header = details.querySelector("summary");
    var joined = chosen.join(",");
    header.dataset.name = joined;
    header.dataset.value = joined;
    details.classList.toggle("has-selection", chosen.length > 0);
    header.querySelector(".formatted-selection").textContent = chosen.length
      ? chosen.join(", ")
      : details.dataset.none || "";
    var caret = header.querySelector(".angle-icon");
    if (full(details)) {
      if (caret) {
        caret.remove();
      }
    } else if (!caret) {
      header
        .querySelector(".select-kit-header-wrapper")
        .insertAdjacentHTML("beforeend", ui().icon("plus").replace("svg-icon", "svg-icon angle-icon"));
    }
    if (details.open) {
      if (full(details)) {
        close(details);
      } else {
        drawBody(details);
      }
    }
    details.dispatchEvent(new Event("change", { bubbles: true }));
  }

  var searchTimer = null;

  function search(details, term) {
    var kind = details.dataset.search;
    var path =
      kind === "tags"
        ? "/tags/filter/search.json?q=" + encodeURIComponent(term) + "&limit=5"
        : "/u/search/users.json?term=" + encodeURIComponent(term) + "&include_groups=false&limit=6";
    return ui()
      .ajax("GET", path)
      .then(function (json) {
        if (kind === "tags") {
          return (json.results || []).map(function (r) {
            if (r.id !== undefined) {
              details.tagIds = details.tagIds || {};
              details.tagIds[r.name || r.text] = r.id;
            }
            return { value: r.name || r.text, label: r.name || r.text };
          });
        }
        return (json.users || []).map(function (u) {
          return { value: u.username, label: u.username, user: u };
        });
      });
  }

  function rowHtml(r, i) {
    var inner = escapeHtml(r.label);
    var cls = "select-kit-row";
    if (r.user) {
      cls += " user-row";
      inner =
        '<img alt="" class="avatar" height="24" width="24" src="' +
        escapeHtml(String(r.user.avatar_template || "").replace("{size}", "24")) +
        '"><span class="username">' +
        escapeHtml(r.user.username) +
        "</span>" +
        (r.user.name ? '<span class="name">' + escapeHtml(r.user.name) + "</span>" : "");
    } else {
      inner = '<span class="name">' + inner + "</span>";
    }
    return (
      '<li aria-checked="false" class="' +
      cls +
      (i === 0 ? " is-highlighted" : "") +
      '" data-index="' +
      i +
      '" data-name="' +
      escapeHtml(r.value) +
      '" data-value="' +
      escapeHtml(r.value) +
      '" role="menuitemradio" tabindex="0" title="' +
      escapeHtml(r.value) +
      '">' +
      inner +
      "</li>"
    );
  }

  function drawBody(details, results) {
    var body = details.querySelector(".select-kit-body");
    var input = body.querySelector(".filter-input");
    var filter = input ? input.value : "";
    var chosen = chosenOf(details);
    var html =
      '<div class="select-kit-filter multi-select-filter is-expanded"' +
      (details.id ? ' id="' + details.id + '-filter"' : "") +
      '><input autocapitalize="off" autocomplete="off" autocorrect="off" class="filter-input" name="filter-input-search" placeholder="' +
      escapeHtml(details.dataset.placeholder || "") +
      '" spellcheck="false" tabindex="0" type="search"></div>';
    if (chosen.length) {
      html +=
        '<div class="selected-content">' +
        chosen
          .map(function (v) {
            return (
              '<button aria-label="' +
              escapeHtml(t("select_kit.delete_item", { name: v })) +
              '" class="btn btn-default selected-choice tag-choice" data-name="' +
              escapeHtml(v) +
              '" data-value="' +
              escapeHtml(v) +
              '" type="button">' +
              ui().icon("xmark") +
              '<span class="d-button-label">' +
              escapeHtml(v) +
              "</span></button>"
            );
          })
          .join("") +
        "</div>";
    }
    var rows = [];
    var trimmed = filter.trim();
    if (details.hasAttribute("data-creatable") && trimmed && chosen.indexOf(trimmed) === -1) {
      rows.push({ value: trimmed, label: t("select_kit.create", { content: trimmed }) });
    }
    (results || []).forEach(function (r) {
      if (chosen.indexOf(r.value) === -1 && r.value !== trimmed) {
        rows.push(r);
      }
    });
    html += '<ul aria-live="polite" class="select-kit-collection" role="menu">' + rows.map(rowHtml).join("") + "</ul>";
    body.innerHTML = html;
    var again = body.querySelector(".filter-input");
    again.value = filter;
    again.focus();
    float(details);
    if (details.dataset.search && results === undefined) {
      clearTimeout(searchTimer);
      searchTimer = setTimeout(function () {
        search(details, trimmed)
          .then(function (found) {
            if (details.open) {
              drawBody(details, found);
            }
          })
          .catch(function () {});
      }, 300);
    }
  }

  function float(details) {
    var body = details.querySelector(".select-kit-body");
    var header = details.querySelector("summary");
    var fui = window.FloatingUIDOM;
    if (!fui) {
      return;
    }
    body.style.position = "fixed";
    body.style.minWidth = Math.round(details.getBoundingClientRect().width) + "px";
    body.style.top = "0";
    body.style.left = "0";
    fui
      .computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [fui.flip({ padding: 3 }), fui.shift({ limiter: fui.limitShift() }), fui.offset(3)],
      })
      .then(function (result) {
        body.style.transform = "translate(" + Math.round(result.x) + "px, " + Math.round(result.y) + "px)";
        body.style.visibility = "visible";
        body.style.pointerEvents = "auto";
      });
  }

  function close(details) {
    details.open = false;
    details.classList.remove("is-expanded");
    var body = details.querySelector(".select-kit-body");
    body.innerHTML = "";
    body.removeAttribute("style");
  }

  function chooserOf(target) {
    return (
      target.closest &&
      target.closest(".admin-contents details.select-kit[data-chooser], .admin-contents details.select-kit[data-category-chooser]")
    );
  }

  // The CategoryChooser's rows: the site's categories by name, filtered.
  function drawCategories(details) {
    var body = details.querySelector(".select-kit-body");
    var input = body.querySelector(".filter-input");
    var filter = input ? input.value.toLowerCase() : "";
    var el = document.getElementById("customize-categories");
    var all = el ? JSON.parse(el.textContent) : [];
    var rows = all.filter(function (c) {
      return !filter || c.name.toLowerCase().indexOf(filter) !== -1;
    });
    body.innerHTML =
      '<div class="select-kit-filter is-expanded"><input autocapitalize="off" autocomplete="off" autocorrect="off" class="filter-input" name="filter-input-search" placeholder="' +
      escapeHtml(t("select_kit.filter_placeholder")) +
      '" spellcheck="false" tabindex="0" type="search"></div><ul aria-live="polite" class="select-kit-collection" role="menu">' +
      rows
        .map(function (c, i) {
          return (
            '<li aria-checked="' +
            (String(c.id) === details.dataset.value) +
            '" class="select-kit-row category-row' +
            (i === 0 ? " is-highlighted" : "") +
            '" data-index="' +
            i +
            '" data-name="' +
            escapeHtml(c.name) +
            '" data-value="' +
            c.id +
            '" role="menuitemradio" tabindex="0" title="' +
            escapeHtml(c.name) +
            '"><div class="category-status">' +
            c.badge +
            "</div></li>"
          );
        })
        .join("") +
      "</ul>";
    var again = body.querySelector(".filter-input");
    again.value = input ? input.value : "";
    again.focus();
    float(details);
  }

  function chooseCategory(details, id) {
    var el = document.getElementById("customize-categories");
    var all = el ? JSON.parse(el.textContent) : [];
    var c = all.find(function (x) {
      return String(x.id) === String(id);
    });
    details.dataset.value = c ? c.id : "";
    var header = details.querySelector("summary");
    header.dataset.name = c ? c.name : "";
    header.dataset.value = c ? c.id : "";
    var choice = header.querySelector(".select-kit-selected-name");
    choice.title = c ? c.name : "";
    choice.querySelector(".name").innerHTML = c ? c.badge : "";
    details.classList.toggle("has-selection", !!c);
    close(details);
    details.dispatchEvent(new Event("change", { bubbles: true }));
  }

  document.addEventListener("click", function (event) {
    var details = chooserOf(event.target);
    document
      .querySelectorAll(
        ".admin-contents details.select-kit[data-chooser][open], .admin-contents details.select-kit[data-category-chooser][open]"
      )
      .forEach(function (d) {
        if (d !== details) {
          close(d);
        }
      });
    if (!details) {
      return;
    }
    event.preventDefault();
    if (details.hasAttribute("data-disabled")) {
      return;
    }
    var category = details.hasAttribute("data-category-chooser");
    var row = event.target.closest(".select-kit-row");
    if (row) {
      if (category) {
        chooseCategory(details, row.dataset.value);
        return;
      }
      details.querySelector(".filter-input").value = "";
      setChosen(details, chosenOf(details).concat([row.dataset.value]));
      return;
    }
    var choice = event.target.closest(".selected-choice");
    if (choice) {
      setChosen(
        details,
        chosenOf(details).filter(function (v) {
          return v !== choice.dataset.value;
        })
      );
      return;
    }
    if (event.target.closest(".select-kit-filter")) {
      return;
    }
    if (details.open) {
      close(details);
      return;
    }
    // A full chooser opens on its selection to take it out.
    details.open = true;
    details.classList.add("is-expanded");
    if (category) {
      drawCategories(details);
    } else {
      drawBody(details);
    }
  });

  document.addEventListener("input", function (event) {
    var details = chooserOf(event.target);
    if (details && event.target.classList.contains("filter-input")) {
      if (details.hasAttribute("data-category-chooser")) {
        drawCategories(details);
      } else {
        drawBody(details);
      }
    }
  });

  document.addEventListener("keydown", function (event) {
    var details = chooserOf(event.target);
    if (!details || !event.target.classList.contains("filter-input")) {
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      var row = details.querySelector(".select-kit-row.is-highlighted");
      if (!row) {
        return;
      }
      if (details.hasAttribute("data-category-chooser")) {
        chooseCategory(details, row.dataset.value);
        return;
      }
      event.target.value = "";
      setChosen(details, chosenOf(details).concat([row.dataset.value]));
    } else if (event.key === "Escape") {
      close(details);
    } else if (event.key === "Backspace" && !event.target.value && details.hasAttribute("data-chooser")) {
      var chosen = chosenOf(details);
      if (chosen.length) {
        setChosen(details, chosen.slice(0, -1));
      }
    }
  });

  // --- FormKit -------------------------------------------------------------

  // A required field's error under it and the summary above the form, as
  // FormKit draws them on submit; true when nothing is missing.
  function required(form, names) {
    var missing = [];
    form.querySelectorAll(".form-kit__errors-summary, .form-kit__meta").forEach(function (el) {
      el.remove();
    });
    names.forEach(function (name) {
      var container = form.querySelector('.form-kit__field[data-name="' + name + '"]');
      var input = container && container.querySelector("input.form-kit__control-input");
      var value = input ? input.value : (chosenOf(container.querySelector("details")) || []).join("");
      container.classList.toggle("has-error", !value);
      if (input) {
        if (value) {
          input.removeAttribute("aria-invalid");
        } else {
          input.setAttribute("aria-invalid", "true");
        }
      }
      if (!value) {
        container
          .querySelector(".form-kit__container-content")
          .insertAdjacentHTML(
            "beforeend",
            '<div class="form-kit__meta"><p aria-live="assertive" class="form-kit__errors"><span>' +
              ui().icon("triangle-exclamation") +
              " " +
              escapeHtml(t("form_kit.errors.required")) +
              "</span></p></div>"
          );
        missing.push({ name: name, title: container.querySelector(".form-kit__container-title span").textContent });
      }
    });
    if (missing.length) {
      form.insertAdjacentHTML(
        "afterbegin",
        '<div aria-live="assertive" class="form-kit__errors-summary"><h2 class="form-kit__errors-summary-title">' +
          ui().icon("triangle-exclamation") +
          " " +
          escapeHtml(t("form_kit.errors_summary_title." + (missing.length === 1 ? "one" : "other"), { count: missing.length })) +
          '</h2><ul class="form-kit__errors-summary-list">' +
          missing
            .map(function (m) {
              return (
                '<li><a href="#control-' +
                escapeHtml(m.name) +
                '" rel="noopener noreferrer">' +
                escapeHtml(m.title) +
                "</a>: " +
                escapeHtml(t("form_kit.errors.required")) +
                "</li>"
              );
            })
            .join("") +
          "</ul></div>"
      );
    }
    return missing.length === 0;
  }

  // The page through the boosted navigation, as a route transition.
  function go(path) {
    var url = ui().base() + path;
    if (!window.htmx) {
      location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // --- the watched words ----------------------------------------------------

  function wordsPage() {
    return document.querySelector(".admin-contents.admin-watched-words");
  }

  function wordsData() {
    var el = document.getElementById("watched-words-data");
    return el ? JSON.parse(el.textContent) : { actions: [], words: [], compiled_regular_expressions: {} };
  }

  var showWords = false;
  var filterTimer = null;

  function filterInput() {
    var root = wordsPage();
    return root && root.querySelector(".inline-form input.no-blur");
  }

  function wordsQuery() {
    var input = filterInput();
    var params = new URLSearchParams();
    if (input && input.value) {
      params.set("filter", input.value);
    }
    return params;
  }

  // The nav and the list drawn again by the server for the filter and the
  // checkbox, and the words' data with them.
  function refreshWords() {
    var root = wordsPage();
    if (!root) {
      return Promise.resolve();
    }
    var params = wordsQuery();
    var shown = new URLSearchParams(params);
    if (showWords) {
      shown.set("show_words", "1");
    }
    var path = location.pathname.replace(ui().base(), "");
    history.replaceState(history.state, "", ui().base() + path + (params.toString() ? "?" + params : ""));
    return fetchPage(path + (shown.toString() ? "?" + shown : "")).then(function (doc) {
      var nav = doc.querySelector(".admin-watched-words .admin-nav");
      var data = doc.getElementById("watched-words-data");
      var current = wordsPage();
      if (!nav || !current) {
        return;
      }
      current.querySelector(".admin-nav").innerHTML = nav.innerHTML;
      document.getElementById("watched-words-data").textContent = data.textContent;
      var detail = current.querySelector(".watched-words-detail");
      detail.querySelectorAll(".show-words-checkbox, .watched-words-list").forEach(function (el) {
        el.remove();
      });
      // After the form, the detail's last child.
      doc.querySelectorAll(".watched-words-detail .show-words-checkbox, .watched-words-detail .watched-words-list").forEach(
        function (el) {
          detail.appendChild(document.importNode(el, true));
        }
      );
      regexpErrors();
    });
  }

  // AdminWatchedWordsActionController#regexpErrors, drawn above the
  // controls: each word's regexp the browser can't compile.
  function regexpErrors() {
    var root = wordsPage();
    if (!root) {
      return;
    }
    var detail = root.querySelector(".watched-words-detail");
    var old = detail.querySelector(".alert.alert-error");
    if (old) {
      old.remove();
    }
    var key = root.dataset.action;
    var errors = [];
    var seen = {};
    wordsData().words.forEach(function (w) {
      if (w.action !== key || !w.regexp) {
        return;
      }
      try {
        new RegExp(w.regexp, "u");
      } catch (e) {
        var id = w.id + ":" + w.word + ":" + e.message;
        if (!seen[id]) {
          seen[id] = true;
          var parts = e.message.split(": ");
          errors.push({ word: w.word, error: parts[parts.length - 1] });
        }
      }
    });
    if (!errors.length) {
      return;
    }
    var alert = document.createElement("div");
    alert.className = "alert alert-error";
    alert.innerHTML =
      "<strong>" +
      escapeHtml(t("admin.watched_words.invalid_regex_multiple")) +
      '</strong><ul class="watched-word-regex-errors">' +
      errors
        .map(function (e) {
          return "<li><strong>" + escapeHtml(e.word) + "</strong>: " + escapeHtml(e.error) + "</li>";
        })
        .join("") +
      "</ul>";
    detail.insertBefore(alert, detail.firstChild);
  }

  function wordsOf(key) {
    return wordsData().words.filter(function (w) {
      return w.action === key;
    });
  }

  function formState(form) {
    var words = chosenOf(form.querySelector("#watched-words"));
    form.querySelector("button[type=submit]").disabled = words.length === 0;
  }

  function setMessage(form, text) {
    var span = form.querySelector(".success-message");
    if (!text) {
      if (span) {
        span.remove();
      }
      return;
    }
    if (!span) {
      span = document.createElement("span");
      span.className = "success-message";
      form.appendChild(span);
    }
    span.textContent = text;
  }

  function setDisabled(form, disabled) {
    form.querySelectorAll("input").forEach(function (i) {
      i.disabled = disabled;
    });
    form.querySelectorAll("details.select-kit").forEach(function (d) {
      d.toggleAttribute("data-disabled", disabled);
      d.classList.toggle("is-disabled", disabled);
    });
  }

  document.addEventListener("change", function (event) {
    var form = event.target.closest && event.target.closest(".admin-watched-words .watched-word-form");
    if (form && event.target.id === "watched-words") {
      formState(form);
      if (chosenOf(event.target).length) {
        setMessage(form, null);
      }
      return;
    }
    var show = event.target.closest && event.target.closest(".admin-watched-words .show-words-checkbox input");
    if (show) {
      showWords = show.checked;
      refreshWords();
    }
  });

  document.addEventListener("input", function (event) {
    if (event.target === filterInput()) {
      clearTimeout(filterTimer);
      filterTimer = setTimeout(refreshWords, 500);
    }
  });

  onSubmit(function (event) {
    var form = event.target.closest(".admin-watched-words .watched-word-form");
    if (!form) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    var root = wordsPage();
    var key = root.dataset.action;
    var words = chosenOf(form.querySelector("#watched-words"));
    if (!words.length || form.dataset.submitting) {
      return;
    }
    // isUniqueWord
    var duplicate = wordsOf(key).some(function (w) {
      if (w.case_sensitive === true) {
        return words.indexOf(w.word) !== -1;
      }
      return words
        .map(function (x) {
          return x.toLowerCase();
        })
        .indexOf(w.word.toLowerCase()) !== -1;
    });
    if (duplicate) {
      setMessage(form, t("admin.watched_words.form.exists"));
      return;
    }
    var caseSensitive = form.querySelector(".case-sensitivity-checkbox input").checked;
    var html = form.querySelector(".html-checkbox input");
    var data = { words: words, action_key: key, case_sensitive: caseSensitive, html: html ? html.checked : false };
    var replacement = form.querySelector("#watched-replacement, #watched-link");
    if (replacement) {
      data.replacement = replacement.value;
    }
    var tags = form.querySelector("#watched-tag");
    if (tags) {
      data.replacement_tags = chosenOf(tags).map(function (name) {
        return { name: name };
      });
    }
    form.dataset.submitting = "1";
    setDisabled(form, true);
    ui()
      .ajax("POST", "/admin/customize/watched_words.json", data)
      .then(function () {
        setChosen(form.querySelector("#watched-words"), []);
        if (tags) {
          setChosen(tags, []);
        }
        if (replacement) {
          replacement.value = "";
        }
        form.querySelectorAll(".checkbox-label input").forEach(function (i) {
          i.checked = false;
        });
        setMessage(form, t("admin.watched_words.form.success"));
        return refreshWords();
      })
      .catch(ui().failed)
      .then(function () {
        delete form.dataset.submitting;
        setDisabled(form, false);
        formState(form);
      });
  });

  document.addEventListener("click", function (event) {
    var root = wordsPage();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var key = root.dataset.action;
    if (event.target.closest(".inline-form .btn-default:not(.menu-toggle)")) {
      filterInput().value = "";
      refreshWords();
      return;
    }
    var del = event.target.closest(".delete-word-record");
    if (del) {
      var id = del.closest(".watched-word").dataset.id;
      ui()
        .ajax("DELETE", "/admin/customize/watched_words/" + id + ".json")
        .then(refreshWords)
        .catch(function (e) {
          Discourse.dialog.alert(
            t("generic_error_with_reason", { error: "http: " + e.status + " - " + (e.message || "") })
          );
        });
      return;
    }
    if (event.target.closest(".clear-all")) {
      Discourse.dialog.yesNoConfirm({
        message: t("admin.watched_words.clear_all_confirm", {
          action: t("admin.watched_words.actions." + key),
        }),
        didConfirm: function () {
          ui()
            .ajax("DELETE", "/admin/customize/watched_words/action/" + key + ".json")
            .then(refreshWords)
            .catch(ui().failed);
        },
      });
      return;
    }
    if (event.target.closest(".watched-word-test")) {
      testModal(key);
      return;
    }
    if (event.target.closest(".menu-toggle")) {
      actionMenu(event.target.closest(".menu-toggle"));
    }
  });

  // The action nav in a DMenu (modalForMobile: the modal's sheet).
  function actionMenu(trigger) {
    var nav = wordsPage().querySelector(".admin-nav ul");
    if (!nav || !Discourse.float) {
      return;
    }
    var content = nav.cloneNode(true);
    Discourse.float.menu(trigger, {
      identifier: "admin-watched-words-action-nav",
      mainClass: "admin-watched-words-action-nav-content",
      content: content,
    });
    var active = content.querySelector("a.active");
    if (active) {
      active.scrollIntoView({ block: "center" });
    }
  }

  // WatchedWordUploader: the file with the action, then the alert.
  document.addEventListener("change", function (event) {
    var input = event.target.closest && event.target.closest(".admin-watched-words .watched-words-uploader input");
    if (!input || !input.files.length) {
      return;
    }
    var body = new FormData();
    body.append("file", input.files[0]);
    body.append("action_key", wordsPage().dataset.action);
    input.disabled = true;
    input.closest("label").classList.add("disabled");
    ui()
      .ajax("POST", "/admin/customize/watched_words/upload.json", body)
      .then(function () {
        Discourse.dialog.alert(t("admin.watched_words.form.upload_successful"));
        return refreshWords();
      })
      .catch(ui().failed)
      .then(function () {
        input.value = "";
        input.disabled = false;
        input.closest("label").classList.remove("disabled");
      });
  });

  // WatchedWordTesting: the matches as the text is typed.
  function testModal(key) {
    var data = wordsData();
    var words = data.words.filter(function (w) {
      return w.action === key;
    });
    var compiled = (data.compiled_regular_expressions || {})[key] || [];
    var name = t("admin.watched_words.actions." + key);
    Discourse.modal.show({
      className: "watched-words-test-modal",
      title: t("admin.watched_words.test.modal_title", { action: name }),
      body: function (body) {
        body.innerHTML =
          "<p>" +
          escapeHtml(t("admin.watched_words.test.description")) +
          '</p><textarea autofocus="autofocus" class="ember-text-area" name="test_value"></textarea><p class="matches">' +
          escapeHtml(t("admin.watched_words.test.no_matches")) +
          "</p>";
        var area = body.querySelector("textarea");
        var out = body.querySelector(".matches");
        area.addEventListener("input", function () {
          var matches = testMatches(key, words, compiled, area.value);
          if (!matches.length) {
            out.innerHTML = escapeHtml(t("admin.watched_words.test.no_matches"));
            return;
          }
          out.innerHTML =
            escapeHtml(t("admin.watched_words.test.found_matches")) +
            "<ul>" +
            matches
              .map(function (m) {
                if (key === "replace" || key === "link") {
                  return (
                    '<li><span class="match">' +
                    escapeHtml(m.match) +
                    '</span> → <span class="replacement">' +
                    escapeHtml(m.replacement) +
                    "</span></li>"
                  );
                }
                if (key === "tag") {
                  return (
                    '<li><span class="match">' +
                    escapeHtml(m.match) +
                    "</span> → " +
                    m.tags
                      .map(function (tag) {
                        return '<span class="tag">' + escapeHtml(tag) + "</span>";
                      })
                      .join("") +
                    "</li>"
                  );
                }
                return "<li>" + escapeHtml(m) + "</li>";
              })
              .join("") +
            "</ul>";
        });
        area.focus();
      },
    });
  }

  function testMatches(key, words, compiled, value) {
    if (!value) {
      return [];
    }
    var matches = [];
    var match;
    if (key === "replace" || key === "link") {
      words.forEach(function (word) {
        try {
          var re = new RegExp(word.regexp, word.case_sensitive ? "gu" : "gui");
          while ((match = re.exec(value)) !== null) {
            matches.push({ match: match[1], replacement: word.replacement });
          }
        } catch (_) {
          // An invalid regexp matches nothing.
        }
      });
      return matches;
    }
    if (key === "tag") {
      var found = new Map();
      words.forEach(function (word) {
        try {
          var re = new RegExp(word.regexp, word.case_sensitive ? "gu" : "gui");
          while ((match = re.exec(value)) !== null) {
            if (!found.has(match[1])) {
              found.set(match[1], new Set());
            }
            var tags = found.get(match[1]);
            word.replacement.split(",").forEach(function (tag) {
              tags.add(tag);
            });
          }
        } catch (_) {
          // An invalid regexp matches nothing.
        }
      });
      return Array.from(found, function (entry) {
        return { match: entry[0], tags: Array.from(entry[1]) };
      });
    }
    var broken = false;
    compiled.forEach(function (entry) {
      try {
        var regexp = Object.keys(entry)[0];
        var re = new RegExp(regexp, entry[regexp].case_sensitive ? "gu" : "gui");
        while ((match = re.exec(value)) !== null) {
          matches.push(match[1] || match[0]);
        }
      } catch (_) {
        broken = true;
      }
    });
    if (broken) {
      matches = [];
      words.forEach(function (word) {
        try {
          var re = new RegExp(word.regexp, word.case_sensitive ? "gu" : "gui");
          while ((match = re.exec(value)) !== null) {
            matches.push(match[1] || match[0]);
          }
        } catch (_) {
          // An invalid regexp matches nothing.
        }
      });
    }
    return matches;
  }

  // --- embedding -----------------------------------------------------------

  function embeddingPage() {
    return document.querySelector(".admin-contents.admin-embedding");
  }

  function embeddingData() {
    var el = document.getElementById("embedding-data");
    return el ? JSON.parse(el.textContent) : {};
  }

  // AdminEmbeddingIndexController#embeddingCode
  function embeddingCode(data) {
    var fullApp = data.full_app
      ? "\n      fullApp: true,\n      embedHeight: '800px',\n      // lazyLoad: false, // disable native lazy loading of the iframe\n      // dynamicHeight: true,\n      // embedMinHeight: '400',\n      // embedMaxHeight: '900',"
      : "";
    return (
      "<div id='discourse-comments'></div>\n  <meta name='discourse-username' content='DISCOURSE_USERNAME'>\n\n  <script type=\"text/javascript\">\n    DiscourseEmbed = {\n      discourseUrl: '" +
      data.base_url +
      "/',\n      discourseEmbedUrl: 'EMBED_URL'," +
      fullApp +
      "\n      // className: 'CLASS_NAME',\n    };\n\n    (function() {\n      var d = document.createElement('script'); d.type = 'text/javascript'; d.async = true;\n      d.src = DiscourseEmbed.discourseUrl + 'javascripts/embed.js';\n      (document.getElementsByTagName('head')[0] || document.getElementsByTagName('body')[0]).appendChild(d);\n    })();\n  </script>"
    );
  }

  function drawSnippet(card, open) {
    var content = card.querySelector(".admin-config-area-card__content");
    var toggle = card.querySelector(".admin-config-area-card__toggle-button");
    toggle.innerHTML = ui().icon(open ? "angle-down" : "angle-right");
    if (!open) {
      if (content) {
        content.remove();
      }
      return;
    }
    if (!content) {
      content = document.createElement("div");
      content.className = "admin-config-area-card__content";
      card.appendChild(content);
    }
    content.innerHTML =
      t("admin.embedding.sample") +
      '<pre><code class="lang-html"></code></pre>';
    var code = content.querySelector("code");
    code.textContent = embeddingCode(embeddingData());
    if (window.hljs) {
      window.hljs.highlightElement(code);
    }
  }

  function hostData(form) {
    var data = {};
    form.querySelectorAll("input.form-kit__control-input").forEach(function (i) {
      data[i.name] = i.value;
    });
    return data;
  }

  document.addEventListener("click", function (event) {
    var root = embeddingPage();
    if (!root || !event.target.closest) {
      return;
    }
    var toggle = event.target.closest(".admin-embedding-index__full-app-toggle .d-toggle-switch__checkbox");
    if (toggle && root.contains(toggle)) {
      var on = toggle.getAttribute("aria-checked") !== "true";
      var set = function (value) {
        toggle.setAttribute("aria-checked", String(value));
        toggle.nextElementSibling.innerHTML = value ? ui().icon("check") : "";
        var data = embeddingData();
        data.full_app = value;
        document.getElementById("embedding-data").textContent = JSON.stringify(data);
        var card = root.querySelector(".admin-embedding-index__code");
        if (card && card.querySelector(".admin-config-area-card__content")) {
          drawSnippet(card, true);
        }
      };
      set(on);
      ui()
        .ajax("PUT", "/admin/site_settings/embed_full_app.json", { embed_full_app: on })
        .catch(function (e) {
          set(!on);
          ui().failed(e);
        });
      return;
    }
    var cardToggle = event.target.closest(".admin-embedding-index__code .admin-config-area-card__toggle-button");
    if (cardToggle) {
      var card = cardToggle.closest(".admin-config-area-card");
      drawSnippet(card, !card.querySelector(".admin-config-area-card__content"));
      return;
    }
    // The host menu's delete (its content lives in the float).
    var del = event.target.closest(".admin-embeddable-host-item__delete");
    if (del) {
      var id = del.closest("[data-host-id]") && del.closest("[data-host-id]").dataset.hostId;
      if (Discourse.float) {
        Discourse.float.close("embedding-host-menu");
      }
      if (!id) {
        return;
      }
      Discourse.dialog.confirm({
        message: t("admin.embedding.confirm_delete"),
        didConfirm: function () {
          ui()
            .ajax("DELETE", "/admin/embeddable_hosts/" + id + ".json")
            .then(function () {
              var row = root.querySelector('tr[data-host-id="' + id + '"]');
              if (row && row.parentNode.children.length > 1) {
                row.remove();
              } else {
                go("/admin/customize/embedding");
              }
            })
            .catch(ui().failed);
        },
      });
    }
  });

  // The host's id on its menu's content, for the delete above.
  ui().menus(function (trigger, content) {
    var row = trigger.closest(".admin-embedding tr[data-host-id]");
    if (row) {
      content.dataset.hostId = row.dataset.hostId;
    }
  });

  onSubmit(function (event) {
    var form = event.target;
    var root = embeddingPage();
    if (!root || !root.contains(form) || !form.classList.contains("form-kit")) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    var request;
    var type = form.dataset.embeddingType;
    if (type) {
      var data = hostData(form);
      data.type = type;
      form.querySelectorAll("input.form-kit__control-checkbox").forEach(function (i) {
        data[i.name] = i.checked;
      });
      var by = form.querySelector('[data-name="embed_by_username"] details');
      if (by) {
        if (!required(form, ["embed_by_username"])) {
          return;
        }
        data.embed_by_username = chosenOf(by)[0];
      }
      request = ui()
        .ajax("PUT", "/admin/customize/embedding.json", { embedding: data })
        .then(function () {
          ui().toast(
            t(
              type === "crawlers"
                ? "admin.embedding.crawler_settings_saved"
                : "admin.embedding.posts_and_topics_settings_saved"
            )
          );
        });
    } else if (form.hasAttribute("data-host-form")) {
      if (!required(form, ["host"])) {
        return;
      }
      var host = hostData(form);
      var id = form.dataset.hostForm;
      var tags = form.querySelector('[data-name="tags"] details');
      var known = JSON.parse(tags.dataset.tagIds || "{}");
      Object.assign(known, tags.tagIds || {});
      host.category_id = form.querySelector('[data-name="category"] details').dataset.value || "";
      host.tags = chosenOf(tags)
        .map(function (name) {
          return known[name];
        })
        .filter(function (v) {
          return v !== undefined;
        });
      host.user = chosenOf(form.querySelector('[data-name="user"] details'))[0] || "";
      request = ui()
        .ajax(id ? "PUT" : "POST", "/admin/embeddable_hosts" + (id ? "/" + id : "") + ".json", {
          embeddable_host: host,
        })
        .then(function () {
          go("/admin/customize/embedding");
        });
    } else {
      return;
    }
    var submit = form.querySelector("button[type=submit]");
    submit.disabled = true;
    request.catch(ui().failed).then(function () {
      submit.disabled = false;
    });
  });

  // --- site texts ----------------------------------------------------------

  function siteTextsIndex() {
    return document.querySelector(".admin-contents.admin-site-text:not(.edit)");
  }

  // The route's query params from the page's controls.
  function siteTextParams() {
    var root = siteTextsIndex();
    var params = new URLSearchParams(location.search);
    params.delete("page");
    var q = root.querySelector(".site-text-search").value;
    if (q) {
      params.set("q", q);
    } else {
      params.delete("q");
    }
    return params;
  }

  var siteTextPage = 0;

  // The list and the filters drawn again for the params (replace: true).
  function refreshSiteTexts(params) {
    var root = siteTextsIndex();
    var query = params.toString();
    history.replaceState(history.state, "", location.pathname + (query ? "?" + query : ""));
    siteTextPage = 0;
    return fetchPage(location.pathname.replace(ui().base(), "") + (query ? "?" + query : "")).then(function (doc) {
      var fresh = doc.querySelector(".admin-site-text .site-texts");
      if (!fresh || !root) {
        return;
      }
      var mine = root.querySelector(".site-texts");
      [".d-filter-controls__toggle-filters", ".site-texts__language"].forEach(function (sel) {
        var a = mine.querySelector(sel);
        var b = fresh.querySelector(sel);
        if (a && b) {
          a.replaceWith(document.importNode(b, true));
        }
      });
      var controls = mine.querySelector(".d-filter-controls");
      var oldDrop = controls.querySelector(".d-filter-controls__dropdowns");
      var newDrop = fresh.querySelector(".d-filter-controls__dropdowns");
      if (oldDrop) {
        oldDrop.remove();
      }
      if (newDrop) {
        controls.appendChild(document.importNode(newDrop, true));
      }
      mine.querySelectorAll(".site-texts__recommended, .site-text-list, template.site-texts-dropdowns").forEach(function (el) {
        el.remove();
      });
      fresh
        .querySelectorAll(".site-texts__recommended, .site-text-list, template.site-texts-dropdowns")
        .forEach(function (el) {
          mine.appendChild(document.importNode(el, true));
        });
      watchSiteTextMore();
    });
  }

  var siteTextObserver = null;

  // DLoadMore: the next page's summaries appended as the end shows.
  function watchSiteTextMore() {
    if (siteTextObserver) {
      siteTextObserver.disconnect();
      siteTextObserver = null;
    }
    var root = siteTextsIndex();
    var sentinel = root && root.querySelector(".load-more-sentinel");
    if (!sentinel || !window.IntersectionObserver) {
      return;
    }
    var busy = false;
    siteTextObserver = new IntersectionObserver(function (entries) {
      if (!entries[0].isIntersecting || busy || !sentinel.hasAttribute("data-has-more")) {
        return;
      }
      busy = true;
      var params = siteTextParams();
      params.set("page", String(siteTextPage + 1));
      fetchPage(location.pathname.replace(ui().base(), "") + "?" + params)
        .then(function (doc) {
          siteTextPage += 1;
          var list = root.querySelector(".site-text-list");
          var loading = list.querySelector(".loading-container");
          doc.querySelectorAll(".site-text").forEach(function (el) {
            list.insertBefore(document.importNode(el, true), loading);
          });
          var more = doc.querySelector(".load-more-sentinel");
          sentinel.toggleAttribute("data-has-more", !!(more && more.hasAttribute("data-has-more")));
        })
        .catch(function () {})
        .then(function () {
          busy = false;
        });
    });
    siteTextObserver.observe(sentinel);
  }

  // A ComboBox over the rows given: the locales, or the themes with the
  // none row (clearable).
  function siteTextCombo(details) {
    var rows;
    if (details.hasAttribute("data-site-text-locale")) {
      var el = document.getElementById("site-text-locales");
      rows = (el ? JSON.parse(el.textContent) : []).map(function (l) {
        return { value: l.value, name: l.name };
      });
    } else {
      rows = [{ value: "", name: details.dataset.none }].concat(
        JSON.parse(details.dataset.siteTextThemes || "[]").map(function (th) {
          return { value: String(th.id), name: th.name };
        })
      );
    }
    var body = details.querySelector(".select-kit-body");
    var input = body.querySelector(".filter-input");
    var filter = input ? input.value.toLowerCase() : "";
    var current = details.querySelector("summary").dataset.value || "";
    body.innerHTML =
      '<div class="select-kit-filter is-expanded"><input autocapitalize="off" autocomplete="off" autocorrect="off" class="filter-input" name="filter-input-search" placeholder="' +
      escapeHtml(t("select_kit.filter_placeholder")) +
      '" spellcheck="false" tabindex="0" type="search"></div><ul aria-live="polite" class="select-kit-collection" role="menu">' +
      rows
        .filter(function (r) {
          return !filter || r.name.toLowerCase().indexOf(filter) !== -1;
        })
        .map(function (r, i) {
          var selected = r.value === current;
          return (
            '<li aria-checked="' +
            selected +
            '" class="select-kit-row' +
            (r.value === "" ? " none" : "") +
            (selected ? " is-selected" : "") +
            (i === 0 ? " is-highlighted" : "") +
            '" data-name="' +
            escapeHtml(r.name) +
            '" data-value="' +
            escapeHtml(r.value) +
            '" role="menuitemradio" tabindex="0" title="' +
            escapeHtml(r.name) +
            '"><span class="name">' +
            escapeHtml(r.name) +
            "</span></li>"
          );
        })
        .join("") +
      "</ul>";
    var again = body.querySelector(".filter-input");
    again.value = input ? input.value : "";
    again.focus();
    float(details);
  }

  function siteTextComboOf(target) {
    return (
      target.closest &&
      target.closest(".admin-site-text details[data-site-text-locale], .admin-site-text details[data-site-text-themes]")
    );
  }

  document.addEventListener("click", function (event) {
    var root = siteTextsIndex();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var combo = siteTextComboOf(event.target);
    root.querySelectorAll("details[data-site-text-locale][open], details[data-site-text-themes][open]").forEach(function (d) {
      if (d !== combo) {
        close(d);
      }
    });
    if (combo) {
      event.preventDefault();
      var row = event.target.closest(".select-kit-row");
      if (row) {
        close(combo);
        var params = siteTextParams();
        var key = combo.hasAttribute("data-site-text-locale") ? "locale" : "theme_id";
        if (row.dataset.value) {
          params.set(key, row.dataset.value);
        } else {
          params.delete(key);
        }
        refreshSiteTexts(params);
        return;
      }
      if (event.target.closest(".select-kit-filter")) {
        return;
      }
      if (combo.open) {
        close(combo);
      } else {
        combo.open = true;
        combo.classList.add("is-expanded");
        siteTextCombo(combo);
      }
      return;
    }
    var toggle = event.target.closest(".d-filter-controls__toggle-filters");
    if (toggle) {
      var controls = root.querySelector(".d-filter-controls");
      var drop = controls.querySelector(".d-filter-controls__dropdowns");
      var template = root.querySelector("template.site-texts-dropdowns");
      if (drop) {
        if (template) {
          drop.remove();
          toggle.setAttribute("aria-expanded", "false");
        }
      } else if (template) {
        controls.appendChild(template.content.firstElementChild.cloneNode(true));
        toggle.setAttribute("aria-expanded", "true");
      }
      return;
    }
    var box = event.target.closest(".filter-options input[type=checkbox]");
    if (box) {
      var p = siteTextParams();
      var name = {
        "toggle-overridden": "overridden",
        "toggle-outdated": "outdated",
        "toggle-only-locale": "onlySelectedLocale",
        "toggle-untranslated": "untranslated",
      }[box.id];
      if (box.checked) {
        p.set(name, "true");
      } else {
        p.delete(name);
      }
      refreshSiteTexts(p);
      return;
    }
    if (event.target.closest(".site-texts__reset-filters")) {
      var reset = siteTextParams();
      ["theme_id", "overridden", "outdated", "untranslated", "onlySelectedLocale"].forEach(function (k) {
        reset.delete(k);
      });
      refreshSiteTexts(reset);
      return;
    }
    var edit = event.target.closest(".site-text-edit");
    if (edit) {
      var id = edit.closest(".site-text").dataset.siteTextId;
      var current = new URLSearchParams(location.search);
      var locale = root.querySelector("details[data-site-text-locale] summary").dataset.value;
      var q = new URLSearchParams();
      q.set("locale", locale);
      if (current.get("theme_id")) {
        q.set("theme_id", current.get("theme_id"));
      }
      go("/admin/customize/site_texts/" + encodeURIComponent(id) + "?" + q);
      return;
    }
    if (event.target.closest("button.reseed:not([data-page-action])")) {
      reseedModal();
    }
  });

  var siteTextTimer = null;
  document.addEventListener("input", function (event) {
    var root = siteTextsIndex();
    if (!root) {
      return;
    }
    if (event.target.classList.contains("site-text-search")) {
      clearTimeout(siteTextTimer);
      siteTextTimer = setTimeout(function () {
        refreshSiteTexts(siteTextParams());
      }, 400);
      return;
    }
    var combo = siteTextComboOf(event.target);
    if (combo && event.target.classList.contains("filter-input")) {
      siteTextCombo(combo);
    }
  });

  // The Reseed modal: the system categories and topics to replace.
  function reseedModal() {
    var close = Discourse.modal.show({
      className: "reseed-modal",
      title: t("admin.reseed.modal.title"),
      subtitle: t("admin.reseed.modal.subtitle"),
      body: function (body) {
        body.innerHTML = '<div class="spinner"></div>';
        ui()
          .ajax("GET", "/admin/customize/reseed.json")
          .then(function (result) {
            var group = function (items, key) {
              if (!items || !items.length) {
                return "";
              }
              return (
                '<fieldset><legend class="options-group-title">' +
                escapeHtml(t("admin.reseed.modal." + key)) +
                "</legend>" +
                items
                  .map(function (item) {
                    return (
                      '<label><input class="option" type="checkbox" data-group="' +
                      key +
                      '" data-id="' +
                      escapeHtml(item.id) +
                      '"' +
                      (item.selected ? " checked" : "") +
                      "><span>" +
                      escapeHtml(item.name) +
                      "</span></label>"
                    );
                  })
                  .join("") +
                "</fieldset>"
              );
            };
            body.innerHTML = group(result.categories, "categories") + "<br>" + group(result.topics, "topics");
          })
          .catch(function (e) {
            body.textContent = e.message;
          });
      },
      footer: function (footer, closeModal) {
        footer.innerHTML =
          '<button class="btn btn-text btn-danger" type="button"><span class="d-button-label">' +
          escapeHtml(t("admin.reseed.modal.replace")) +
          '</span></button><button class="btn btn-flat d-modal-cancel" type="button"><span class="d-button-label">' +
          escapeHtml(t("cancel")) +
          "</span></button>";
        footer.querySelector(".d-modal-cancel").addEventListener("click", closeModal);
        footer.querySelector(".btn-danger").addEventListener("click", function () {
          var modal = footer.closest(".d-modal");
          var ids = function (key) {
            return Array.prototype.map.call(
              modal.querySelectorAll('input[data-group="' + key + '"]:checked'),
              function (i) {
                return i.dataset.id;
              }
            );
          };
          ui()
            .ajax("POST", "/admin/customize/reseed.json", {
              category_ids: ids("categories"),
              topic_ids: ids("topics"),
            })
            .then(closeModal)
            .catch(function () {
              var flash = modal.querySelector(".d-modal__body");
              flash.insertAdjacentHTML(
                "beforebegin",
                '<div id="modal-alert" role="alert" class="alert alert-error">' + escapeHtml(t("generic_error")) + "</div>"
              );
            });
        });
      },
    });
    return close;
  }

  // The edit page: the buffered value against the saved one, the
  // interpolation keys' pills, save, revert and dismiss outdated.
  function siteTextEdit() {
    return document.querySelector(".admin-contents.admin-site-text.edit .edit-site-text");
  }

  function siteTextData() {
    var el = document.getElementById("site-text-data");
    return el ? JSON.parse(el.textContent) : null;
  }

  var siteTextSaved = null;
  var siteTextCursor = null;

  function drawKeys(edit, value, keys) {
    var old = edit.querySelector(".interpolation-keys");
    if (!keys) {
      if (old) {
        old.remove();
      }
      return;
    }
    var known = keys;
    var used = {};
    var invalid = [];
    (value.match(/%\{(\w+)\}/g) || []).forEach(function (m) {
      var k = m.slice(2, -1);
      if (known.indexOf(k) !== -1) {
        used[k] = true;
      } else if (invalid.indexOf(k) === -1) {
        invalid.push(k);
      }
    });
    var html = "";
    known.forEach(function (k) {
      html +=
        '<button class="interpolation-keys__pill' +
        (used[k] ? " is-used" : "") +
        '" title="' +
        escapeHtml(t(used[k] ? "admin.site_text.interpolation_key_used" : "admin.site_text.interpolation_key_insert")) +
        '" type="button">' +
        escapeHtml(k) +
        "</button>";
    });
    invalid.forEach(function (k) {
      html +=
        '<span class="interpolation-keys__pill is-invalid" title="' +
        escapeHtml(t("admin.site_text.interpolation_key_invalid")) +
        '">' +
        escapeHtml(k) +
        "</span>";
    });
    if (!html) {
      if (old) {
        old.remove();
      }
      return;
    }
    if (!old) {
      old = document.createElement("div");
      old.className = "interpolation-keys";
      edit.querySelector(".save-button").insertAdjacentElement("beforebegin", old);
    }
    old.innerHTML = html;
  }

  function siteTextState(edit) {
    var area = edit.querySelector("#site-text-value");
    edit.querySelector(".save-changes").disabled = area.value === siteTextSaved;
    var data = siteTextData();
    drawKeys(edit, area.value, data && data.site_text.interpolation_keys);
  }

  function setSaved(edit, saved) {
    var label = edit.querySelector(".save-button .saved");
    if (saved && !label) {
      edit.querySelector(".save-changes").insertAdjacentHTML("afterend", '<span class="saved">' + escapeHtml(t("saved")) + "</span>");
    } else if (!saved && label) {
      label.remove();
    }
  }

  function setRevert(edit, canRevert) {
    var revert = edit.querySelector(".revert-site-text");
    if (canRevert && !revert) {
      edit
        .querySelector(".save-button")
        .insertAdjacentHTML(
          "beforeend",
          '<button class="btn revert-site-text" type="button"><span class="d-button-label">' +
            escapeHtml(t("admin.site_text.revert")) +
            "</span></button>"
        );
    } else if (!canRevert && revert) {
      revert.remove();
    }
  }

  // DExpandingTextArea: autosize's height, the content's with the borders.
  function expand(area) {
    area.style.height = "auto";
    area.style.height = area.scrollHeight + (area.offsetHeight - area.clientHeight) + "px";
  }

  document.addEventListener("input", function (event) {
    if (event.target.matches && event.target.matches(".admin-contents textarea.--expandable")) {
      expand(event.target);
    }
    var edit = siteTextEdit();
    if (edit && event.target.id === "site-text-value") {
      siteTextState(edit);
    }
  });

  document.addEventListener(
    "focusout",
    function (event) {
      if (siteTextEdit() && event.target.id === "site-text-value") {
        siteTextCursor = { start: event.target.selectionStart, end: event.target.selectionEnd };
      }
    },
    true
  );

  document.addEventListener("click", function (event) {
    var edit = siteTextEdit();
    if (!edit || !edit.contains(event.target)) {
      return;
    }
    var data = siteTextData();
    var id = data.site_text.id;
    var locale = data.locale;
    var area = edit.querySelector("#site-text-value");
    var pill = event.target.closest("button.interpolation-keys__pill");
    if (pill) {
      var token = "%{" + pill.textContent.trim() + "}";
      var start = siteTextCursor ? siteTextCursor.start : area.value.length;
      var end = siteTextCursor ? siteTextCursor.end : area.value.length;
      area.focus();
      area.setSelectionRange(start, end);
      document.execCommand("insertText", false, token);
      siteTextCursor = { start: area.selectionStart, end: area.selectionStart };
      siteTextState(edit);
      return;
    }
    if (event.target.closest(".save-changes")) {
      var save = edit.querySelector(".save-changes");
      save.disabled = true;
      ui()
        .ajax("PUT", "/admin/customize/site_texts/" + encodeURIComponent(id) + ".json", {
          site_text: { value: area.value, locale: locale },
        })
        .then(function (json) {
          siteTextSaved = area.value;
          if (json && json.site_text) {
            data.site_text = Object.assign(data.site_text, json.site_text);
            document.getElementById("site-text-data").textContent = JSON.stringify(data);
            setRevert(edit, json.site_text.can_revert);
          }
          setSaved(edit, true);
          siteTextState(edit);
        })
        .catch(function (e) {
          siteTextState(edit);
          ui().failed(e);
        });
      return;
    }
    if (event.target.closest(".revert-site-text")) {
      setSaved(edit, false);
      Discourse.dialog.yesNoConfirm({
        message: t("admin.site_text.revert_confirm"),
        didConfirm: function () {
          ui()
            .ajax(
              "DELETE",
              "/admin/customize/site_texts/" + encodeURIComponent(id) + ".json?locale=" + encodeURIComponent(locale)
            )
            .then(function (json) {
              area.value = json.site_text.value;
              siteTextSaved = area.value;
              data.site_text = Object.assign(data.site_text, json.site_text);
              document.getElementById("site-text-data").textContent = JSON.stringify(data);
              setRevert(edit, json.site_text.can_revert);
              siteTextState(edit);
            })
            .catch(ui().failed);
        },
      });
      return;
    }
    if (event.target.closest(".dismiss-outdated")) {
      ui()
        .ajax(
          "PUT",
          "/admin/customize/site_texts/" + encodeURIComponent(id) + "/dismiss_outdated.json?locale=" + encodeURIComponent(locale)
        )
        .then(function () {
          var outdated = event.target.closest(".outdated");
          if (outdated) {
            outdated.remove();
          }
        })
        .catch(ui().failed);
    }
  });

  // --- email templates -----------------------------------------------------

  // AdminEmailTemplatesEditController: the subject and body against the
  // template's, the interpolation keys over both, save and revert; DEditor's
  // toolbar and its preview.
  function emailTemplate() {
    return document.querySelector(".admin-email-templates.edit .email-template");
  }

  function emailTemplateData() {
    var el = document.getElementById("email-template-data");
    return el ? JSON.parse(el.textContent) : null;
  }

  var emailSaved = null;
  var emailActive = null;
  var emailCursor = null;

  function emailFields(root) {
    return {
      subject: root.querySelector(".email-template__subject"),
      body: root.querySelector(".email-template__body textarea"),
    };
  }

  function emailState(root) {
    var f = emailFields(root);
    var subject = f.subject ? f.subject.value : null;
    var body = f.body ? f.body.value : null;
    root.querySelector(".save-changes").disabled =
      emailSaved && subject === emailSaved.subject && body === emailSaved.body;
    var data = emailTemplateData();
    drawKeys(root, (subject || "") + " " + (body || ""), data && data.template && data.template.interpolation_keys);
  }

  var markdownRenderer = null;

  // The markdown crate as WebAssembly, as the composer loads it.
  function renderer() {
    var data = emailTemplateData();
    if (!markdownRenderer) {
      markdownRenderer = Promise.all([
        WebAssembly.instantiateStreaming(fetch(data.preview_wasm), {}),
        fetch(data.preview_settings, { credentials: "same-origin" }).then(function (r) {
          return r.text();
        }),
      ]).then(function (loaded) {
        var wasm = loaded[0].instance.exports;
        var call = function (fn, text) {
          var bytes = new TextEncoder().encode(text);
          var ptr = wasm.alloc(bytes.length);
          new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
          var status = fn(ptr, bytes.length);
          wasm.dealloc(ptr, bytes.length);
          var out = new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.output_ptr(), wasm.output_len()));
          if (status !== 0) {
            throw new Error(out);
          }
          return out;
        };
        call(wasm.configure, loaded[1]);
        return function (raw) {
          return call(wasm.preview, raw);
        };
      });
    }
    return markdownRenderer;
  }

  function emailPreview(root) {
    var f = emailFields(root);
    var preview = root.querySelector(".d-editor-preview");
    if (!f.body || !preview || !window.WebAssembly) {
      return;
    }
    renderer()
      .then(function (render) {
        try {
          preview.innerHTML = render(f.body.value);
        } catch (e) {
          preview.textContent = e.message;
        }
      })
      .catch(function () {});
  }

  // The toolbar's text changes, as d-editor's applySurround and
  // applyList make them on the selection (or their placeholder).
  function surround(area, head, tail, placeholder) {
    var start = area.selectionStart;
    var end = area.selectionEnd;
    var selected = area.value.slice(start, end) || placeholder;
    area.focus();
    area.setSelectionRange(start, end);
    document.execCommand("insertText", false, head + selected + tail);
    area.setSelectionRange(start + head.length, start + head.length + selected.length);
  }

  function prefixLines(area, prefix, placeholder) {
    var start = area.selectionStart;
    var end = area.selectionEnd;
    var selected = area.value.slice(start, end) || placeholder;
    var lines = selected.split("\n").map(function (line, i) {
      return (typeof prefix === "function" ? prefix(i) : prefix) + line;
    });
    area.focus();
    area.setSelectionRange(start, end);
    document.execCommand("insertText", false, lines.join("\n"));
  }

  function toolbarMenu(trigger, items, onPick) {
    if (!Discourse.float) {
      return;
    }
    var content = document.createElement("div");
    content.className = "fk-d-menu__inner-content";
    content.innerHTML =
      '<ul class="dropdown-menu">' +
      items
        .map(function (item, i) {
          return (
            '<li class="dropdown-menu__item"><button class="btn btn-icon-text btn-transparent" data-index="' +
            i +
            '" type="button">' +
            ui().icon(item.icon) +
            '<span class="d-button-label">' +
            escapeHtml(item.label) +
            "</span></button></li>"
          );
        })
        .join("") +
      "</ul>";
    trigger.setAttribute("aria-expanded", "true");
    Discourse.float.menu(trigger, {
      identifier: trigger.dataset.identifier,
      mainClass: trigger.dataset.identifier + "-content",
      content: content,
      onClose: function () {
        trigger.setAttribute("aria-expanded", "false");
      },
    });
    content.addEventListener("click", function (event) {
      var button = event.target.closest("button[data-index]");
      if (button) {
        Discourse.float.close(trigger.dataset.identifier);
        onPick(items[Number(button.dataset.index)]);
      }
    });
  }

  document.addEventListener("input", function (event) {
    var root = emailTemplate();
    if (!root || !root.contains(event.target)) {
      return;
    }
    emailState(root);
    if (event.target.classList.contains("d-editor-input")) {
      emailPreview(root);
    }
  });

  document.addEventListener(
    "focusin",
    function (event) {
      var root = emailTemplate();
      if (root && root.contains(event.target) && /^(TEXTAREA|INPUT)$/.test(event.target.tagName)) {
        emailActive = event.target;
      }
    },
    true
  );

  document.addEventListener(
    "focusout",
    function (event) {
      var root = emailTemplate();
      if (root && root.contains(event.target) && /^(TEXTAREA|INPUT)$/.test(event.target.tagName)) {
        emailCursor = { start: event.target.selectionStart, end: event.target.selectionEnd };
      }
    },
    true
  );

  document.addEventListener("click", function (event) {
    var root = emailTemplate();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var data = emailTemplateData();
    var f = emailFields(root);
    var area = f.body;
    var tool = event.target.closest(".toolbar__button");
    if (tool && area) {
      if (tool.classList.contains("bold")) {
        surround(area, "**", "**", t("composer.bold_text"));
      } else if (tool.classList.contains("italic")) {
        surround(area, "*", "*", t("composer.italic_text"));
      } else if (tool.classList.contains("blockquote")) {
        prefixLines(area, "> ", t("composer.blockquote_text"));
      } else if (tool.classList.contains("code")) {
        var sel = area.value.slice(area.selectionStart, area.selectionEnd);
        if (sel.indexOf("\n") !== -1) {
          surround(area, "```\n", "\n```", "");
        } else if (sel) {
          surround(area, "`", "`", "");
        } else {
          prefixLines(area, "    ", t("composer.code_text"));
        }
      } else if (tool.classList.contains("heading")) {
        toolbarMenu(
          tool,
          [1, 2, 3, 4]
            .map(function (n) {
              return { icon: "discourse-h" + n, label: t("composer.heading_level_n", { levelNumber: n }), prefix: "#".repeat(n) + " " };
            })
            .concat([{ icon: "discourse-paragraph", label: t("composer.heading_level_paragraph"), prefix: "" }]),
          function (item) {
            prefixLines(area, item.prefix, "");
            emailState(root);
            emailPreview(root);
          }
        );
        return;
      } else if (tool.classList.contains("list")) {
        toolbarMenu(
          tool,
          [
            { icon: "list-ul", label: t("composer.ulist_title"), ordered: false },
            { icon: "list-ol", label: t("composer.olist_title"), ordered: true },
          ],
          function (item) {
            prefixLines(
              area,
              function (i) {
                return item.ordered ? i + 1 + ". " : "- ";
              },
              t("composer.list_item")
            );
            emailState(root);
            emailPreview(root);
          }
        );
        return;
      } else if (tool.classList.contains("link")) {
        ui().unsupported("the insert hyperlink modal");
        return;
      } else if (tool.classList.contains("emoji")) {
        ui().unsupported("the emoji picker in the email template editor");
        return;
      }
      emailState(root);
      emailPreview(root);
      return;
    }
    var pill = event.target.closest("button.interpolation-keys__pill");
    if (pill) {
      var target = emailActive;
      if (!target) {
        return;
      }
      var token = "%{" + pill.textContent.trim() + "}";
      var start = emailCursor ? emailCursor.start : target.value.length;
      var end = emailCursor ? emailCursor.end : target.value.length;
      target.focus();
      target.setSelectionRange(start, end);
      document.execCommand("insertText", false, token);
      emailCursor = { start: target.selectionStart, end: target.selectionStart };
      emailState(root);
      emailPreview(root);
      return;
    }
    var id = data.template.id;
    if (event.target.closest(".save-changes")) {
      var payload = {};
      if (f.subject) {
        payload.subject = f.subject.value;
      }
      if (f.body) {
        payload.body = f.body.value;
      }
      setSaved(root, false);
      ui()
        .ajax("PUT", "/admin/email/templates/" + id, { email_template: payload })
        .then(function (json) {
          emailSaved = { subject: f.subject ? f.subject.value : null, body: f.body ? f.body.value : null };
          if (json && json.email_template) {
            data.template = json.email_template;
            document.getElementById("email-template-data").textContent = JSON.stringify(data);
            setEmailRevert(root, json.email_template.can_revert);
          }
          setSaved(root, true);
          emailState(root);
        })
        .catch(ui().failed);
      return;
    }
    if (event.target.closest(".revert-email-template")) {
      setSaved(root, false);
      Discourse.dialog.yesNoConfirm({
        title: t("admin.customize.email_templates.revert_confirm"),
        didConfirm: function () {
          ui()
            .ajax("DELETE", "/admin/email/templates/" + id)
            .then(function (json) {
              var tpl = json.email_template;
              if (f.subject && typeof tpl.subject === "string") {
                f.subject.value = tpl.subject;
              }
              if (f.body && typeof tpl.body === "string") {
                f.body.value = tpl.body;
              }
              emailSaved = { subject: f.subject ? f.subject.value : null, body: f.body ? f.body.value : null };
              data.template = tpl;
              document.getElementById("email-template-data").textContent = JSON.stringify(data);
              setEmailRevert(root, tpl.can_revert);
              emailState(root);
              emailPreview(root);
            })
            .catch(ui().failed);
        },
      });
    }
  });

  function setEmailRevert(root, canRevert) {
    var revert = root.querySelector(".revert-email-template");
    if (canRevert && !revert) {
      root
        .querySelector(".save-button")
        .insertAdjacentHTML(
          "beforeend",
          '<button class="btn revert-email-template" type="button"><span class="d-button-label">' +
            escapeHtml(t("admin.customize.email_templates.revert")) +
            "</span></button>"
        );
    } else if (!canRevert && revert) {
      revert.remove();
    }
  }

  // --- color palettes --------------------------------------------------------

  function palettesPage() {
    return document.querySelector(".admin-contents.color-palettes .color-palette__list, .admin-contents.color-palettes .d-filter-controls")
      ? document.querySelector(".admin-contents.color-palettes")
      : null;
  }

  function paletteEditor() {
    return document.querySelector(".admin-contents.color-palettes form[data-palette-id]");
  }

  function palettesData() {
    var el = document.getElementById("color-palettes-data");
    return el ? JSON.parse(el.textContent) : { palettes: [], extras: {} };
  }

  function saveScheme(id, data) {
    return ui().ajax(id ? "PUT" : "POST", "/admin/color_schemes" + (id ? "/" + id : "") + ".json", {
      json: { color_scheme: data },
    });
  }

  // DFilterControls over the list: the text against the name and theme
  // name, the type dropdown, both in the URL (replace).
  function filterPalettes() {
    var root = palettesPage();
    if (!root) {
      return;
    }
    var input = root.querySelector(".d-filter-controls__input");
    var select = root.querySelector(".d-filter-controls__dropdown");
    var text = input ? input.value.toLowerCase() : "";
    var kind = select ? select.value : "all";
    var params = new URLSearchParams(location.search);
    if (text) {
      params.set("filter", input.value);
    } else {
      params.delete("filter");
    }
    if (kind !== "all") {
      params.set("type", kind);
    } else {
      params.delete("type");
    }
    history.replaceState(history.state, "", location.pathname + (params.toString() ? "?" + params : ""));
    // The page drawn again for the params: the list or the no results.
    fetchPage(location.pathname.replace(ui().base(), "") + (params.toString() ? "?" + params : "")).then(function (doc) {
      var mine = palettesPage();
      var fresh = doc.querySelector(".admin-contents.color-palettes");
      if (!mine || !fresh) {
        return;
      }
      mine.querySelectorAll(".color-palette__list, .d-filter-controls__no-results").forEach(function (el) {
        el.remove();
      });
      var controls = mine.querySelector(".d-filter-controls");
      fresh.querySelectorAll(".color-palette__list, .d-filter-controls__no-results").forEach(function (el) {
        controls.insertAdjacentElement("afterend", document.importNode(el, true));
      });
      var reset = controls.querySelector(".d-filter-controls__reset");
      if (reset) {
        reset.disabled = !text && kind === "all";
      }
      if (select) {
        select.classList.toggle("--active", kind !== "all");
      }
    });
  }

  var paletteTimer = null;

  document.addEventListener("input", function (event) {
    if (palettesPage() && event.target.classList.contains("d-filter-controls__input")) {
      clearTimeout(paletteTimer);
      paletteTimer = setTimeout(filterPalettes, 250);
    }
  });

  document.addEventListener("change", function (event) {
    if (palettesPage() && event.target.classList.contains("d-filter-controls__dropdown")) {
      filterPalettes();
    }
  });

  // setDefaultColorScheme: the palette made the default theme's light or
  // dark one (a base palette: the current one's flag taken off).
  function setDefaultPalette(item, mode) {
    var data = palettesData();
    var theme = data.extras.default_theme;
    var id = Number(item.dataset.paletteId);
    var scheme = data.palettes.find(function (p) {
      return p.id === id;
    });
    var field = mode === "light" ? "default_light_on_theme" : "default_dark_on_theme";
    var themeField = mode === "light" ? "color_scheme_id" : "dark_color_scheme_id";
    var request;
    if (!scheme.is_base) {
      var change = {};
      change[field] = true;
      request = saveScheme(id, change);
    } else {
      var current = theme && theme[themeField];
      var off = {};
      off[field] = false;
      request = current > 0 ? saveScheme(current, off) : Promise.resolve();
    }
    request
      .then(function () {
        if (!scheme.is_builtin_default) {
          ui().toast(
            t("admin.customize.colors.set_default_success", {
              schemeName: scheme.name,
              themeName: theme ? theme.name : "",
            })
          );
        }
        go(location.pathname.replace(ui().base(), "") + location.search);
      })
      .catch(function () {
        Discourse.dialog.alert({ message: t("admin.customize.colors.default_error") });
      });
  }

  document.addEventListener("click", function (event) {
    var root = palettesPage();
    if (!root || !event.target.closest) {
      return;
    }
    if (root.contains(event.target)) {
      var trigger = event.target.closest(".color-palette__controls .fk-d-menu__trigger");
      if (trigger && Discourse.float) {
        var template = trigger.nextElementSibling;
        var content = template.content.firstElementChild.cloneNode(true);
        content.dataset.paletteId = trigger.closest("[data-palette-id]").dataset.paletteId;
        trigger.setAttribute("aria-expanded", "true");
        Discourse.float.menu(trigger, {
          identifier: "color-palette-menu",
          mainClass: "color-palette-menu-content",
          content: content,
          onClose: function () {
            trigger.setAttribute("aria-expanded", "false");
          },
        });
        return;
      }
      if (event.target.closest(".create-new-palette:not([data-page-action])")) {
        newPaletteModal();
        return;
      }
      if (event.target.closest(".d-filter-controls__reset, .d-filter-controls__reset-button")) {
        var input = root.querySelector(".d-filter-controls__input");
        var select = root.querySelector(".d-filter-controls__dropdown");
        if (input) {
          input.value = "";
        }
        if (select) {
          select.value = "all";
        }
        filterPalettes();
        return;
      }
    }
    var menu = event.target.closest("[data-palette-id] .dropdown-menu, .color-palette-menu-content");
    var holder = menu && menu.closest("[data-palette-id]");
    var button = event.target.closest(".dropdown-menu button");
    if (!holder || !button) {
      return;
    }
    Discourse.float.close("color-palette-menu");
    var id = holder.dataset.paletteId;
    if (button.classList.contains("palette-user-selectable")) {
      var item = root.querySelector('li[data-palette-id="' + id + '"]');
      var on = item.dataset.userSelectable !== "true";
      saveScheme(id, { user_selectable: on })
        .then(function () {
          go(location.pathname.replace(ui().base(), "") + location.search);
        })
        .catch(ui().failed);
    } else if (button.classList.contains("btn-palette-default")) {
      setDefaultPalette(holder, button.dataset.mode);
    } else if (button.classList.contains("palette-delete")) {
      Discourse.dialog.deleteConfirm({
        title: t("admin.customize.colors.delete_confirm"),
        didConfirm: function () {
          ui()
            .ajax("DELETE", "/admin/color_schemes/" + id)
            .then(function () {
              go(location.pathname.replace(ui().base(), "") + location.search);
            })
            .catch(ui().failed);
        },
      });
    }
  });

  // ColorSchemeSelectBaseModal: a base palette copied as a new one.
  function newPaletteModal() {
    var data = palettesData();
    var palettes = data.palettes.filter(function (base) {
      return !(
        base.id < 0 &&
        data.palettes.some(function (p) {
          return p.name === base.name && p.base_scheme_id === base.base_scheme_id && p.id > 0;
        })
      );
    });
    Discourse.modal.show({
      className: "create-color-palette",
      title: t("admin.customize.colors.select_base.title"),
      body: function (body) {
        body.innerHTML =
          escapeHtml(t("admin.customize.colors.select_base.description")) +
          ' <select class="select-base-palette d-native-select">' +
          palettes
            .map(function (p) {
              return '<option value="' + escapeHtml(p.id) + '">' + escapeHtml(p.name) + "</option>";
            })
            .join("") +
          "</select>";
      },
      footer: function (footer, close) {
        footer.innerHTML =
          '<button class="btn btn-icon-text btn-primary" type="button">' +
          ui().icon("plus") +
          '<span class="d-button-label">' +
          escapeHtml(t("admin.customize.new")) +
          "</span></button>";
        footer.querySelector("button").addEventListener("click", function () {
          var select = footer.closest(".d-modal").querySelector(".select-base-palette");
          var base = palettes.find(function (p) {
            return String(p.id) === select.value;
          });
          saveScheme(null, {
            name: t("admin.customize.colors.new_name"),
            base_scheme_id: base.id,
            colors: base.colors.map(function (c) {
              return { name: c.name, hex: c.hex };
            }),
          })
            .then(function (json) {
              close();
              go("/admin/config/colors/" + json.id);
            })
            .catch(ui().failed);
        });
      },
    });
  }

  // The editor: what changed since the palette loaded or was saved.
  var paletteChanged = {};

  function paletteData() {
    var el = document.getElementById("color-palette-data");
    return el ? JSON.parse(el.textContent) : null;
  }

  function markUnsaved(form) {
    var unsaved = Object.keys(paletteChanged).some(function (k) {
      return paletteChanged[k];
    });
    var card = form.querySelector(".admin-config-color-palettes__save-card");
    var note = card.querySelector(".admin-config-color-palettes__unsaved-changes");
    if (unsaved && !note) {
      card.insertAdjacentHTML(
        "afterbegin",
        '<span class="admin-config-color-palettes__unsaved-changes">' +
          escapeHtml(t("admin.config_areas.color_palettes.unsaved_changes")) +
          "</span>"
      );
    } else if (!unsaved && note) {
      note.remove();
    }
  }

  function setColor(item, hex) {
    hex = hex.replace(/^#/, "");
    var full = hex.length === 3 ? hex.split("").map(function (c) { return c + c; }).join("") : hex;
    item.querySelector(".color-palette-editor__input").value = "#" + full;
    item.querySelector(".color-palette-editor__text-input").value = full;
    item.dataset.hex = hex;
    var revert = item.querySelector(".color-palette-editor__revert");
    if (revert) {
      revert.classList.toggle("--hidden", !item.dataset.defaultHex || item.dataset.defaultHex === hex);
    }
  }

  function toggleSwitch(button, on) {
    button.setAttribute("aria-checked", String(on));
    button.nextElementSibling.innerHTML = on ? ui().icon("check") : "";
  }

  function nameEditing(form, editing) {
    var control = form.querySelector(".admin-config-color-palettes__name-control");
    var data = paletteData();
    if (editing) {
      control.outerHTML =
        '<div class="form-kit__container form-kit__field form-kit__field-input --full" data-control-type="input" data-name="name" id="control-name"><div class="form-kit__container-content"><div class="admin-config-color-palettes__name-control"><div class="form-kit__control-input-wrapper"><input class="form-kit__control-input" id="control-name-input" name="name" type="text" value="' +
        escapeHtml(data.name) +
        '"></div><button class="btn no-text btn-icon btn-primary admin-config-color-palettes__save-name" type="button">' +
        ui().icon("check") +
        '<span aria-hidden="true">&#8203;</span></button><button class="btn no-text btn-icon btn-flat admin-config-color-palettes__cancel-edit-name" type="button">' +
        ui().icon("xmark") +
        '<span aria-hidden="true">&#8203;</span></button></div></div></div>';
      form.querySelector("#control-name-input").focus();
    } else {
      var field = form.querySelector('.form-kit__field[data-name="name"]');
      field.outerHTML =
        '<div class="admin-config-color-palettes__name-control"><h2 class="admin-config-color-palettes__name">' +
        escapeHtml(data.name) +
        '</h2><button class="btn no-text btn-icon btn-flat admin-config-color-palettes__edit-name" type="button">' +
        ui().icon("pencil") +
        '<span aria-hidden="true">&#8203;</span></button></div>';
      paletteChanged.name = false;
      markUnsaved(form);
    }
  }

  function savePalette(form, nameOnly) {
    var data = paletteData();
    var nameInput = form.querySelector("#control-name-input");
    if (nameInput && !nameInput.value.trim()) {
      return;
    }
    var payload = { name: nameInput ? nameInput.value : data.name };
    if (!nameOnly) {
      payload.user_selectable =
        form.querySelector('[data-name="user_selectable"] .d-toggle-switch__checkbox').getAttribute("aria-checked") === "true";
      ["default_light_on_theme", "default_dark_on_theme"].forEach(function (field) {
        if (paletteChanged[field]) {
          payload[field] =
            form.querySelector('[data-name="' + field + '"] .d-toggle-switch__checkbox').getAttribute("aria-checked") === "true";
        }
      });
      payload.base_scheme_id = data.base_scheme_id;
      payload.colors = [];
      form.querySelectorAll(".color-palette-editor__colors-item").forEach(function (item) {
        var original = data.colors.find(function (c) {
          return c.name === item.dataset.colorName;
        });
        var hex = item.querySelector(".color-palette-editor__text-input").value;
        if (!original || original.hex !== hex) {
          payload.colors.push({ name: item.dataset.colorName, hex: hex });
        }
      });
    }
    var submit = form.querySelector("button[type=submit]");
    submit.disabled = true;
    return saveScheme(data.id, payload)
      .then(function () {
        data.name = payload.name;
        if (!nameOnly) {
          data.user_selectable = payload.user_selectable;
          (payload.colors || []).forEach(function (change) {
            var c = data.colors.find(function (x) {
              return x.name === change.name;
            });
            if (c) {
              c.hex = change.hex;
            }
          });
          paletteChanged = {};
        }
        document.getElementById("color-palette-data").textContent = JSON.stringify(data);
        if (nameInput) {
          nameEditing(form, false);
        }
        paletteChanged.name = false;
        markUnsaved(form);
        ui().toast(t("saved"));
      })
      .catch(ui().failed)
      .then(function () {
        submit.disabled = false;
      });
  }

  onSubmit(function (event) {
    var form = paletteEditor();
    if (!form || event.target !== form) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    savePalette(form, false);
  });

  document.addEventListener("click", function (event) {
    var form = paletteEditor();
    if (!form || !form.parentNode.contains(event.target)) {
      return;
    }
    var data = paletteData();
    var toggle = event.target.closest(".form-kit__field-toggle .d-toggle-switch__checkbox");
    if (toggle) {
      var on = toggle.getAttribute("aria-checked") !== "true";
      toggleSwitch(toggle, on);
      paletteChanged[toggle.closest("[data-name]").dataset.name] = true;
      markUnsaved(form);
      return;
    }
    if (event.target.closest(".admin-config-color-palettes__edit-name")) {
      nameEditing(form, true);
      return;
    }
    if (event.target.closest(".admin-config-color-palettes__cancel-edit-name")) {
      nameEditing(form, false);
      return;
    }
    if (event.target.closest(".admin-config-color-palettes__save-name")) {
      savePalette(form, true);
      return;
    }
    var revert = event.target.closest(".color-palette-editor__revert");
    if (revert) {
      var item = revert.closest(".color-palette-editor__colors-item");
      setColor(item, item.dataset.defaultHex);
      paletteChanged.colors = true;
      markUnsaved(form);
      return;
    }
    if (event.target.closest(".copy-to-clipboard")) {
      var light = {};
      form.querySelectorAll(".color-palette-editor__colors-item").forEach(function (it) {
        light[it.dataset.colorName] = it.querySelector(".color-palette-editor__text-input").value;
      });
      navigator.clipboard
        .writeText(JSON.stringify({ name: data.name, light: light }, null, 2))
        .then(function () {
          ui().toast(t("admin.config_areas.color_palettes.copied_to_clipboard"));
        })
        .catch(function () {
          Discourse.dialog.alert(t("admin.config_areas.color_palettes.copy_to_clipboard_error"));
        });
      return;
    }
    if (event.target.closest(".duplicate-palette")) {
      saveScheme(null, {
        name: t("admin.config_areas.color_palettes.copy_of", { name: data.name }),
        colors: data.colors.map(function (c) {
          return { name: c.name, hex: c.hex };
        }),
      })
        .then(function (json) {
          go("/admin/config/colors/" + json.id);
          ui().toast(t("admin.config_areas.color_palettes.copy_created", { name: data.name }));
        })
        .catch(ui().failed);
      return;
    }
    if (event.target.closest(".delete-palette")) {
      Discourse.dialog.deleteConfirm({
        title: t("admin.config_areas.color_palettes.delete_confirm"),
        didConfirm: function () {
          ui()
            .ajax("DELETE", "/admin/color_schemes/" + data.id)
            .then(function () {
              go("/admin/config/colors");
            })
            .catch(ui().failed);
        },
      });
    }
  });

  document.addEventListener("input", function (event) {
    var form = paletteEditor();
    if (!form || !form.contains(event.target)) {
      return;
    }
    var item = event.target.closest(".color-palette-editor__colors-item");
    if (item && event.target.classList.contains("color-palette-editor__input")) {
      setColor(item, event.target.value);
      paletteChanged.colors = true;
      markUnsaved(form);
    } else if (event.target.id === "control-name-input") {
      paletteChanged.name = true;
      markUnsaved(form);
    }
  });

  // The hex field: hexadecimal only, Enter moves on to the next color, a
  // change takes a valid #rgb or #rrggbb.
  document.addEventListener("keypress", function (event) {
    var form = paletteEditor();
    var input = event.target;
    if (!form || !input.classList || !input.classList.contains("color-palette-editor__text-input")) {
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      if (input.value.length !== 6 && input.value.length !== 3) {
        input.parentNode.parentNode.classList.add("--invalid");
        Discourse.dialog.alert(t("admin.config_areas.color_palettes.invalid_color_length"));
        return;
      }
      var next = form.querySelector(
        '.color-palette-editor__text-input[data-position="' + (Number(input.dataset.position) + 1) + '"]'
      );
      if (next) {
        next.focus();
      }
      return;
    }
    if (!/^[0-9A-Fa-f]$/.test(event.key)) {
      event.preventDefault();
      Discourse.dialog.alert(t("admin.config_areas.color_palettes.illegal_character_in_color"));
    }
  });

  document.addEventListener("change", function (event) {
    var form = paletteEditor();
    var input = event.target;
    if (!form || !input.classList || !input.classList.contains("color-palette-editor__text-input")) {
      return;
    }
    var picker = input.closest(".color-palette-editor__picker");
    if (!/^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.test(input.value)) {
      picker.classList.add("--invalid");
      Discourse.dialog.alert(t("admin.config_areas.color_palettes.invalid_color_length"));
      return;
    }
    picker.classList.remove("--invalid");
    setColor(input.closest(".color-palette-editor__colors-item"), input.value);
    paletteChanged.colors = true;
    markUnsaved(form);
  });

  // --- AceEditor -------------------------------------------------------------
  //
  // div.ace[data-ace-mode] (src/admin_code_view.rs ace_editor): Ace loaded
  // from the bundle once, the editor made as AceEditor's setupAce makes it,
  // its value the element's text. A change fires `ace-change` on the
  // element (detail: the value); the editor is the element's aceEditor.

  var acePromise = null;

  function loadAce(src) {
    if (!acePromise) {
      acePromise = new Promise(function (resolve, reject) {
        if (window.ace) {
          resolve(window.ace);
          return;
        }
        var script = document.createElement("script");
        script.src = src;
        script.onload = function () {
          resolve(window.ace);
        };
        script.onerror = reject;
        document.head.appendChild(script);
      });
    }
    return acePromise;
  }

  function setupAce(el) {
    if (el.aceEditor) {
      return Promise.resolve(el.aceEditor);
    }
    return loadAce(el.dataset.aceSrc).then(function (ace) {
      if (el.aceEditor || !document.contains(el)) {
        return el.aceEditor;
      }
      ace.config.set("useWorker", false);
      var editor = ace.edit(el);
      editor.setShowPrintMargin(false);
      editor.setOptions({ fontSize: "14px" });
      var session = editor.getSession();
      session.setMode("ace/mode/" + (el.dataset.aceMode || "css"));
      editor.on("change", function () {
        el.dispatchEvent(new CustomEvent("ace-change", { bubbles: true, detail: session.getValue() }));
      });
      editor.$blockScrolling = Infinity;
      editor.renderer.setScrollMargin(10, 10);
      var disabled = el.dataset.aceDisabled === "true";
      editor.setOptions({
        readOnly: disabled,
        highlightActiveLine: !disabled,
        highlightGutterLine: !disabled,
      });
      editor.container.parentNode.parentNode.setAttribute("data-disabled", String(disabled));
      var scheme = getComputedStyle(document.body).getPropertyValue("--scheme-type").trim();
      editor.setTheme("ace/theme/" + (scheme === "dark" ? "chaos" : "chrome"));
      el.aceEditor = editor;
      return editor;
    });
  }

  function setupAces() {
    document.querySelectorAll(".admin-contents div.ace[data-ace-mode]").forEach(function (el) {
      setupAce(el).catch(function () {});
    });
  }

  // --- email style -----------------------------------------------------------

  function emailStyleRoot() {
    return document.querySelector(".admin-contents.admin-customize-email-style");
  }

  function emailStyleData() {
    var el = document.getElementById("email-style-data");
    return el ? JSON.parse(el.textContent) : null;
  }

  var emailStyleChanged = false;

  function emailStyleButtons(root) {
    var data = emailStyleData();
    var editor = root.querySelector("div.ace").aceEditor;
    var value = editor ? editor.getValue() : data.style[data.field];
    var buttons = root.querySelectorAll(".admin-footer .buttons button");
    // Reset, inside the editor's footer; Save, the page's.
    buttons[0].disabled = value === data.style["default_" + data.field];
    buttons[1].disabled = !emailStyleChanged;
  }

  document.addEventListener("ace-change", function (event) {
    var root = emailStyleRoot();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var data = emailStyleData();
    data.style[data.field] = event.detail;
    document.getElementById("email-style-data").textContent = JSON.stringify(data);
    emailStyleChanged = true;
    emailStyleButtons(root);
  });

  document.addEventListener("click", function (event) {
    var root = emailStyleRoot();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var data = emailStyleData();
    var button = event.target.closest(".admin-footer .buttons button");
    if (!button) {
      return;
    }
    var editor = root.querySelector("div.ace").aceEditor;
    if (button.classList.contains("btn-default")) {
      Discourse.dialog.yesNoConfirm({
        message: t("admin.customize.email_style.reset_confirm", {
          fieldName: t("admin.customize.email_style." + data.field),
        }),
        didConfirm: function () {
          if (editor) {
            editor.getSession().setValue(data.style["default_" + data.field]);
          }
        },
      });
      return;
    }
    if (button.classList.contains("btn-primary")) {
      button.disabled = true;
      button.querySelector(".d-button-label").textContent = t("saving");
      ui()
        .ajax("PUT", "/admin/customize/email_style", {
          email_style: { html: data.style.html, css: data.style.css },
        })
        .catch(function (e) {
          var errors = e.json && e.json.errors;
          Discourse.dialog.alert(
            errors
              ? t("admin.customize.email_style.save_error_with_reason", { error: [].concat(errors).join(". ") })
              : t("generic_error")
          );
        })
        .then(function () {
          emailStyleChanged = false;
          button.querySelector(".d-button-label").textContent = t("admin.customize.save");
          emailStyleButtons(root);
        });
    }
  });

  // The edit route's willTransition: leaving with unsaved changes asks to
  // discard or stay (ahead of the boosted navigation, in capture).
  document.addEventListener(
    "click",
    function (event) {
      var root = emailStyleRoot();
      var link = event.target.closest && event.target.closest("a[href]");
      if (!root || !link || !emailStyleChanged || link.classList.contains("active")) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      Discourse.dialog.confirm({
        message: t("admin.customize.theme.unsaved_changes_alert"),
        confirmLabel: t("admin.customize.theme.discard"),
        cancelLabel: t("admin.customize.theme.stay"),
        didConfirm: function () {
          emailStyleChanged = false;
          go(link.getAttribute("href").replace(ui().base(), ""));
        },
      });
    },
    true
  );

  // --- form templates ----------------------------------------------------------

  function formTemplatesRoot() {
    return document.querySelector(".admin-contents.admin-customize-form-templates");
  }

  function formTemplatesData() {
    var el = document.getElementById("form-templates-data");
    return el ? JSON.parse(el.textContent) : { templates: [] };
  }

  // templateFormFields' structures.
  function fieldStructure(type) {
    var p = function (k) {
      return t("admin.form_templates.field_placeholders." + k);
    };
    var id = "  id: " + p("id");
    var label = '    label: "' + p("label") + '"';
    var placeholder = '    placeholder: "' + p("placeholder") + '"';
    var validations = "  validations:\n    # " + p("validations");
    var choices =
      '  choices:\n    - "' + p("choices.first") + '"\n    - "' + p("choices.second") + '"\n    - "' + p("choices.third") + '"';
    var noneLabel = '    none_label: "' + p("none_label") + '"';
    switch (type) {
      case "checkbox":
        return ["- type: checkbox", id, "  attributes:", label, validations].join("\n");
      case "input":
        return ["- type: input", id, "  attributes:", label, placeholder, validations].join("\n");
      case "textarea":
        return ["- type: textarea", id, "  attributes:", label, placeholder, validations].join("\n");
      case "dropdown":
        return ["- type: dropdown", id, choices, "  attributes:", noneLabel, label, validations].join("\n");
      case "upload":
        return [
          "- type: upload",
          id,
          "  attributes:",
          '    file_types: ".jpg, .png, .gif"',
          "    allow_multiple: false",
          label,
          validations,
        ].join("\n");
      case "multiselect":
        return ["- type: multi-select", id, choices, "  attributes:", noneLabel, label, validations].join("\n");
      case "tagchooser":
        return [
          "- type: tag-chooser",
          id,
          '  tag_group: ""',
          "  attributes:",
          noneLabel,
          label,
          "    multiple: true",
          validations,
        ].join("\n");
      case "composer":
        return ["- type: composer", id, "  attributes:", label].join("\n");
    }
    return "";
  }

  function formTemplateState(root) {
    var form = root.querySelector(".form-templates__form");
    if (!form) {
      return;
    }
    var name = form.querySelector(".form-templates__form-name-input").value;
    var editor = form.querySelector("div.ace").aceEditor;
    var content = editor ? editor.getValue() : form.querySelector("div.ace").textContent;
    var empty = !name.length || !content.length;
    form.querySelector(".form-templates__preview-button").disabled = empty;
    form.querySelector(".footer-buttons .btn-primary").disabled = empty;
  }

  document.addEventListener("ace-change", function (event) {
    var root = formTemplatesRoot();
    if (root && root.contains(event.target)) {
      formTemplateState(root);
    }
  });

  document.addEventListener("input", function (event) {
    var root = formTemplatesRoot();
    if (root && event.target.classList.contains("form-templates__form-name-input")) {
      formTemplateState(root);
    }
  });

  function validationsModal() {
    var prefix = "admin.form_templates.validations_modal.";
    Discourse.modal.show({
      className: "form-templates__validation-options admin-form-template-validation-options-modal",
      title: t(prefix + "modal_title"),
      body: function (body) {
        body.innerHTML =
          "<table><thead><tr>" +
          ["key", "type", "description"]
            .map(function (h) {
              return "<th>" + escapeHtml(t(prefix + "table_headers." + h)) + "</th>";
            })
            .join("") +
          "</tr></thead><tbody>" +
          ["required", "minimum", "maximum", "pattern", "type"]
            .map(function (v) {
              var k = function (part) {
                return escapeHtml(t(prefix + "validations." + v + "." + part));
              };
              return "<tr><td><pre>" + k("key") + "</pre></td><td>" + k("type") + "</td><td>" + k("description") + "</td></tr>";
            })
            .join("") +
          "</tbody></table>";
      },
    });
  }

  function deleteTemplate(id) {
    Discourse.dialog.deleteConfirm({
      title: t("admin.form_templates.delete_confirm"),
      didConfirm: function () {
        ui()
          .ajax("DELETE", "/admin/customize/form-templates/" + id + ".json")
          .then(function () {
            go("/admin/customize/form-templates");
          })
          .catch(ui().failed);
      },
    });
  }

  // CustomizeFormTemplateView: the template's YAML, edit, delete, close.
  function viewTemplateModal(template) {
    Discourse.modal.show({
      className: "customize-form-template-view-modal",
      title: template.name,
      body: function (body) {
        body.innerHTML =
          '<div class="control-group"><button class="btn btn-default form-templates__preview-toggle" type="button"><span class="d-button-label">' +
          escapeHtml(t("admin.form_templates.view_template.toggle_preview")) +
          '</span></button></div><pre><code class="language-yaml"></code></pre>';
        body.querySelector("code").textContent = template.template;
        body.querySelector(".form-templates__preview-toggle").addEventListener("click", function () {
          ui().unsupported("the form template preview (Psych's YAML.dump)");
        });
      },
      footer: function (footer, close) {
        footer.innerHTML =
          '<button class="btn btn-icon-text btn-primary" type="button">' +
          ui().icon("pencil") +
          '<span class="d-button-label">' +
          escapeHtml(t("admin.form_templates.view_template.edit")) +
          '</span></button><button class="btn btn-icon-text btn-danger" type="button">' +
          ui().icon("trash-can") +
          '<span class="d-button-label">' +
          escapeHtml(t("admin.form_templates.view_template.delete")) +
          '</span></button><button class="btn btn-default" type="button"><span class="d-button-label">' +
          escapeHtml(t("admin.form_templates.view_template.close")) +
          "</span></button>";
        var buttons = footer.querySelectorAll("button");
        buttons[0].addEventListener("click", function () {
          close();
          go("/admin/customize/form-templates/" + template.id);
        });
        buttons[1].addEventListener("click", function () {
          close();
          deleteTemplate(template.id);
        });
        buttons[2].addEventListener("click", close);
      },
    });
  }

  document.addEventListener("click", function (event) {
    var root = formTemplatesRoot();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var data = formTemplatesData();
    var row = event.target.closest("tr[data-template-id]");
    if (row) {
      var template = data.templates.find(function (x) {
        return String(x.id) === row.dataset.templateId;
      });
      if (event.target.closest(".btn-view-template")) {
        viewTemplateModal(template);
      } else if (event.target.closest(".btn-edit-template")) {
        go("/admin/customize/form-templates/" + template.id);
      } else if (event.target.closest(".btn-delete-template")) {
        deleteTemplate(template.id);
      }
      return;
    }
    var form = root.querySelector(".form-templates__form");
    if (!form) {
      return;
    }
    var id = form.dataset.templateId;
    var aceEl = form.querySelector("div.ace");
    var quick = event.target.closest("[data-field-type]");
    if (quick && aceEl.aceEditor) {
      var structure = fieldStructure(quick.dataset.fieldType);
      var current = aceEl.aceEditor.getValue();
      aceEl.aceEditor.getSession().setValue(current.length ? current + "\n" + structure : structure);
      formTemplateState(root);
      return;
    }
    if (event.target.closest(".form-templates__validations-modal-button")) {
      validationsModal();
      return;
    }
    if (event.target.closest(".form-templates__preview-button")) {
      var params = new URLSearchParams();
      params.set("name", form.querySelector(".form-templates__form-name-input").value);
      params.set("template", aceEl.aceEditor ? aceEl.aceEditor.getValue() : "");
      if (id) {
        params.set("id", id);
      }
      ui()
        .ajax("GET", "/admin/customize/form-templates/preview.json?" + params)
        .then(function () {
          ui().unsupported("the form template preview (Psych's YAML.dump)");
        })
        .catch(ui().failed);
      return;
    }
    var footer = event.target.closest(".footer-buttons button");
    if (!footer) {
      return;
    }
    if (footer.classList.contains("btn-primary")) {
      var payload = {
        name: form.querySelector(".form-templates__form-name-input").value,
        template: aceEl.aceEditor ? aceEl.aceEditor.getValue() : aceEl.textContent,
      };
      footer.disabled = true;
      ui()
        .ajax(id ? "PUT" : "POST", "/admin/customize/form-templates" + (id ? "/" + id : "") + ".json", payload)
        .then(function () {
          go("/admin/customize/form-templates");
        })
        .catch(function (e) {
          footer.disabled = false;
          ui().failed(e);
        });
    } else if (footer.classList.contains("btn-danger")) {
      deleteTemplate(id);
    } else {
      go("/admin/customize/form-templates");
    }
  });

  // --- robots.txt ----------------------------------------------------------

  // AdminCustomizeRobotsTxtController: the buffered text against the
  // saved one, save and reset, DSaveControls' saved label.
  var robotsSaved = null;

  function robotsEdit() {
    return document.querySelector(".admin-customize-robots-txt .robots-txt-edit");
  }

  function robotsState(edit, saved) {
    var save = edit.querySelector(".save-changes");
    var reset = edit.querySelector(".save-button .btn-default");
    var label = edit.querySelector(".save-button .saved");
    if (saved) {
      if (!label) {
        label = document.createElement("span");
        label.className = "saved";
        label.textContent = t("saved");
        save.insertAdjacentElement("afterend", label);
      }
    } else if (label && saved === false) {
      label.remove();
    }
    save.disabled = edit.querySelector(".robots-txt-input").value === robotsSaved;
    return reset;
  }

  function setOverridden(edit, overridden) {
    var notice = edit.querySelector(".overridden");
    if (overridden && !notice) {
      notice = document.createElement("div");
      notice.className = "overridden";
      notice.textContent = t("admin.customize.robots.overridden");
      edit.querySelector(".robots-txt-input").insertAdjacentElement("beforebegin", notice);
    } else if (!overridden && notice) {
      notice.remove();
    }
    edit.querySelector(".save-button .btn-default").disabled = !overridden;
  }

  function robotsBusy(edit, busy) {
    var save = edit.querySelector(".save-changes");
    save.querySelector(".d-button-label").textContent = t(busy ? "saving" : "save");
    if (busy) {
      save.disabled = true;
    }
  }

  document.addEventListener("input", function (event) {
    var edit = robotsEdit();
    if (edit && event.target.classList.contains("robots-txt-input")) {
      robotsState(edit);
    }
  });

  document.addEventListener("click", function (event) {
    var edit = robotsEdit();
    if (!edit || !edit.contains(event.target)) {
      return;
    }
    var save = event.target.closest(".save-changes");
    var reset = event.target.closest(".save-button .btn-default");
    if (!save && !reset) {
      return;
    }
    var input = edit.querySelector(".robots-txt-input");
    robotsBusy(edit, true);
    robotsState(edit, false);
    var request = save
      ? ui().ajax("PUT", "/admin/customize/robots.json", { robots_txt: input.value })
      : ui().ajax("DELETE", "/admin/customize/robots.json");
    request
      .then(function (json) {
        if (reset) {
          input.value = json.robots_txt;
        }
        robotsSaved = input.value;
        setOverridden(edit, save ? json.overridden : false);
        robotsBusy(edit, false);
        robotsState(edit, true);
      })
      .catch(function (e) {
        robotsBusy(edit, false);
        robotsState(edit);
        ui().failed(e);
      });
  });

  Discourse.onPage(function () {
    ui().strings("admin-customize-strings");
    var edit = robotsEdit();
    robotsSaved = edit ? edit.querySelector(".robots-txt-input").value : null;
    var text = siteTextEdit();
    siteTextSaved = text ? text.querySelector("#site-text-value").value : null;
    siteTextCursor = null;
    siteTextPage = 0;
    document.querySelectorAll(".admin-contents textarea.--expandable").forEach(expand);
    paletteChanged = {};
    emailStyleChanged = false;
    setupAces();
    var email = emailTemplate();
    emailActive = null;
    emailCursor = null;
    if (email) {
      var f = emailFields(email);
      emailSaved = { subject: f.subject ? f.subject.value : null, body: f.body ? f.body.value : null };
      emailPreview(email);
    }
    if (siteTextsIndex()) {
      watchSiteTextMore();
    }
    showWords = false;
    regexpErrors();
  });
})();
