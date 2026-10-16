// The admin web hooks pages in the browser: WebhooksList's delete and
// its next pages, WebhooksForm (the event chooser, the category, tag and
// group filters, the payload URL and secret checks, save), and a hook's
// page: delete, WebhookEvents' status filter, ping, redeliver failed, the
// events' request and response details and redelivery, their next pages
// and the incoming ones the bus announces. The times are drawn in the
// browser's zone, as moment draws them.
(function () {
  "use strict";

  if (!Discourse.once("admin-web-hooks")) {
    return;
  }

  var ui = Discourse.adminUi;
  var t = ui.t;
  var esc = ui.escapeHtml;

  function page() {
    return document.querySelector(".admin-contents.admin-web-hooks");
  }

  function hookPath(id) {
    return "/admin/api/web_hooks/" + encodeURIComponent(id);
  }

  function go(path) {
    window.location = ui.base() + path;
  }

  function count(key, n) {
    return t(key + (n === 1 ? ".one" : ".other"), { count: n });
  }

  // WebhookItem's menu: Events is a route button; Delete confirms.
  ui.menus(function (trigger, content, menu) {
    var row = trigger.closest("[data-web-hook-id]");
    if (!row) {
      return;
    }
    content.addEventListener("click", function (event) {
      if (!event.target.closest(".admin-web-hook__delete")) {
        return;
      }
      if (menu && menu.close) {
        menu.close();
      }
      Discourse.dialog.deleteConfirm({
        message: t("admin.web_hooks.delete_confirm"),
        didConfirm: function () {
          ui.ajax("DELETE", hookPath(row.dataset.webHookId))
            .then(function () {
              location.reload();
            })
            .catch(ui.failed);
        },
      });
    });
  });

  // WebhookStatus for a row the next page brings.
  var STATUS_CLASSES = ["--inactive", "--critical", "--success", "--inactive"];

  function hookRow(hook, statuses) {
    var status = (statuses || []).find(function (s) {
      return s.id === hook.last_delivery_status;
    }) || { name: "inactive" };
    var desc = hook.wildcard_web_hook
      ? "*"
      : (hook.web_hook_event_types || [])
          .map(function (ty) {
            return ty.name.toLowerCase() + "_event";
          })
          .join(", ");
    var template = document.querySelector(".admin-web_hooks__items tbody tr");
    var row = template.cloneNode(true);
    row.dataset.webHookId = hook.id;
    var links = row.querySelectorAll("a");
    links[0].href = ui.base() + hookPath(hook.id);
    links[1].href = ui.base() + hookPath(hook.id) + "/edit";
    links[1].textContent = hook.payload_url;
    var label = row.querySelector(".status-label");
    label.className = "status-label " + (STATUS_CLASSES[hook.last_delivery_status - 1] || "--inactive");
    label.querySelector(".status-label-text").textContent = t("admin.web_hooks.delivery_status." + status.name);
    row.querySelector(".key-description").textContent = desc;
    row.querySelector(".key-controls button[data-href]").dataset.href = hookPath(hook.id) + "/edit";
    return row;
  }

  // WebhookEvent
  function eventRow(e) {
    var template = document.querySelector(".web-hook-events ul li");
    var li = template.cloneNode(true);
    li.dataset.event = JSON.stringify(e);
    fillEvent(li, e);
    var details = li.querySelector(".details");
    if (details) {
      details.remove();
    }
    return li;
  }

  function fillEvent(li, e) {
    var status = li.querySelector(".status");
    var cls = !e.status ? "" : e.status >= 200 && e.status <= 299 ? "text-successful" : "text-danger";
    status.innerHTML = '<span class="' + cls + '">' + esc(e.status) + "</span>";
    li.querySelector(".event-id").textContent = e.id;
    var stamp = li.querySelector(".timestamp");
    stamp.dataset.time = new Date(e.created_at).getTime();
    localTime(stamp);
    var seconds = Math.floor(e.duration / 10.0) / 100.0;
    li.querySelector(".completion").textContent = t(
      "admin.web_hooks.events.completed_in" + (seconds === 1 ? ".one" : ".other"),
      { count: seconds }
    );
  }

  function pad(n) {
    return String(n).padStart(2, "0");
  }

  // moment(created_at).format("YYYY-MM-DD HH:mm:ss")
  function localTime(el) {
    var d = new Date(Number(el.dataset.time));
    if (isNaN(d.getTime())) {
      return;
    }
    el.textContent =
      d.getFullYear() +
      "-" +
      pad(d.getMonth() + 1) +
      "-" +
      pad(d.getDate()) +
      " " +
      pad(d.getHours()) +
      ":" +
      pad(d.getMinutes()) +
      ":" +
      pad(d.getSeconds());
  }

  // ensureJSON + plainJSON / prettyJSON (lib/formatter.js)
  function ensureJSON(v) {
    if (typeof v === "string") {
      try {
        return JSON.parse(v);
      } catch (_) {
        return {};
      }
    }
    return v || {};
  }

  function plainJSON(v) {
    var json = ensureJSON(v);
    return Object.keys(json)
      .map(function (k) {
        return k + ": " + json[k];
      })
      .join("\n");
  }

  function prettyJSON(v) {
    try {
      return JSON.stringify(ensureJSON(v), null, 2);
    } catch (_) {
      return String(v);
    }
  }

  function toggleDetails(li, kind) {
    var e = JSON.parse(li.dataset.event);
    var current = li.dataset.expanded || "";
    var existing = li.querySelector(".details");
    if (existing) {
      existing.remove();
    }
    var buttons = li.querySelectorAll(".col.actions button");
    var setIcon = function (button, name) {
      var svg = button.querySelector("svg");
      svg.setAttribute("class", svg.getAttribute("class").replace(/d-icon-[a-z-]+/, "d-icon-" + name));
      svg.querySelector("use").setAttribute("href", "#" + name);
    };
    setIcon(buttons[0], "ellipsis-vertical");
    setIcon(buttons[1], "ellipsis-vertical");
    if (current === kind) {
      delete li.dataset.expanded;
      return;
    }
    var headers;
    var body;
    var label;
    if (kind === "request") {
      var h = Object.assign({ "Request URL": e.request_url, "Request method": "POST" }, ensureJSON(e.headers));
      headers = plainJSON(h);
      body = prettyJSON(e.payload);
      label = t("admin.web_hooks.events.payload");
      setIcon(buttons[0], "ellipsis");
    } else {
      headers = plainJSON(e.response_headers);
      body = e.response_body || "";
      label = t("admin.web_hooks.events.body");
      setIcon(buttons[1], "ellipsis");
    }
    li.dataset.expanded = kind;
    li.insertAdjacentHTML(
      "beforeend",
      '<div class="details"><h3>' +
        esc(t("admin.web_hooks.events.headers")) +
        "</h3><pre><code>" +
        esc(headers) +
        "</code></pre><h3>" +
        esc(label) +
        "</h3><pre><code>" +
        esc(body) +
        "</code></pre></div>"
    );
  }

  function failedIds() {
    return Array.prototype.map
      .call(document.querySelectorAll(".web-hook-events li[data-event]"), function (li) {
        return JSON.parse(li.dataset.event);
      })
      .filter(function (e) {
        return (e.status < 200 || e.status > 299) && e.status !== 0;
      })
      .map(function (e) {
        return e.id;
      });
  }

  function onShowClick(event, listing) {
    var id = listing.dataset.webHookId;
    var li = event.target.closest("li[data-event]");
    if (li && event.target.closest(".web-hook-event__request")) {
      toggleDetails(li, "request");
    } else if (li && event.target.closest(".web-hook-event__response")) {
      toggleDetails(li, "response");
    } else if (li && event.target.closest(".web-hook-event__redeliver")) {
      var e = JSON.parse(li.dataset.event);
      Discourse.dialog.yesNoConfirm({
        message: t("admin.web_hooks.events.redeliver_confirm"),
        didConfirm: function () {
          ui.ajax("POST", hookPath(id) + "/events/" + e.id + "/redeliver")
            .then(function (json) {
              var updated = Object.assign(e, json.web_hook_event);
              li.dataset.event = JSON.stringify(updated);
              fillEvent(li, updated);
            })
            .catch(ui.failed);
        },
      });
    } else if (event.target.closest(".webhook-events__ping-button")) {
      var ping = event.target.closest("button");
      ping.disabled = true;
      ui.ajax("POST", hookPath(id) + "/ping").catch(function (err) {
        ping.disabled = false;
        ui.failed(err);
      });
    } else if (event.target.closest(".web-hook-events__redeliver-failed")) {
      var button = event.target.closest("button");
      var ids = failedIds();
      if (!ids.length) {
        Discourse.dialog.alert(t("admin.web_hooks.events.no_events_to_redeliver"));
        button.disabled = true;
        return;
      }
      Discourse.dialog.yesNoConfirm({
        message: count("admin.web_hooks.events.redeliver_failed_confirm", ids.length),
        didConfirm: function () {
          ui.ajax("POST", hookPath(id) + "/events/failed_redeliver", { event_ids: ids })
            .then(function (response) {
              if (response && response.event_ids && response.event_ids.length) {
                response.event_ids.forEach(function (eid) {
                  document.querySelectorAll(".web-hook-events li[data-event]").forEach(function (row) {
                    if (JSON.parse(row.dataset.event).id === eid) {
                      row.querySelector(".status").innerHTML = ui.icon("arrows-rotate");
                    }
                  });
                });
              } else {
                Discourse.dialog.alert(t("admin.web_hooks.events.no_events_to_redeliver"));
              }
            })
            .catch(ui.failed)
            .then(function () {
              button.disabled = true;
            });
        },
      });
    } else if (event.target.closest(".admin-webhooks__delete-button")) {
      Discourse.dialog.deleteConfirm({
        message: t("admin.web_hooks.delete_confirm"),
        didConfirm: function () {
          ui.ajax("DELETE", hookPath(id))
            .then(function () {
              go("/admin/api/web_hooks");
            })
            .catch(ui.failed);
        },
      });
    }
  }

  // WebhooksForm: the event chooser shown for individual events.
  function syncWildcard(form) {
    var individual = form.querySelector('input[name="wildcard"][value="individual"]');
    var selector = form.querySelector(".event-selector");
    var template = form.querySelector("template.event-selector-template");
    var fieldset = form.querySelector('fieldset[name="wildcard"]');
    if (individual.checked && !selector) {
      if (template) {
        individual.closest(".form-kit__field-radio").after(template.content.firstElementChild.cloneNode(true));
      }
    } else if (!individual.checked && selector) {
      if (!template) {
        template = document.createElement("template");
        template.className = "event-selector-template";
        fieldset.appendChild(template);
      }
      template.content.replaceChildren(selector);
    }
  }

  function setError(field, message) {
    var content = field.querySelector(".form-kit__container-content");
    var meta = content.querySelector(".form-kit__meta");
    if (meta) {
      meta.remove();
    }
    field.classList.toggle("has-error", !!message);
    if (message) {
      content.insertAdjacentHTML(
        "beforeend",
        '<div class="form-kit__meta"><p aria-live="assertive" class="form-kit__errors"><span>' +
          ui.icon("triangle-exclamation") +
          " " +
          esc(message) +
          "</span></p></div>"
      );
    }
  }

  function validUrl(v) {
    try {
      new URL(v);
      return true;
    } catch (_) {
      return false;
    }
  }

  function submit(form) {
    var url = form.querySelector('input[name="payload_url"]');
    var secret = form.querySelector('input[name="secret"]');
    var urlError = !url.value ? t("form_kit.errors.required") : !validUrl(url.value) ? t("form_kit.errors.invalid_url") : null;
    var secretError =
      secret.value && secret.value.length < 12 ? count("form_kit.errors.too_short", 12) : null;
    setError(url.closest(".form-kit__field"), urlError);
    setError(secret.closest(".form-kit__field"), secretError);
    if (urlError || secretError) {
      return;
    }
    var root = page();
    var isNew = root.classList.contains("new");
    var wildcard = form.querySelector('input[name="wildcard"][value="wildcard"]').checked;
    var selector = form.querySelector(".event-selector") ||
      (form.querySelector("template.event-selector-template") || { content: document.createDocumentFragment() }).content;
    var types = Array.prototype.filter
      .call(selector.querySelectorAll('input[name="event-choice"]'), function (box) {
        return box.checked;
      })
      .map(function (box) {
        return Number(box.dataset.id);
      });
    var items = function (chooser) {
      try {
        return JSON.parse((chooser && chooser.dataset.items) || "[]");
      } catch (_) {
        return [];
      }
    };
    var categories = items(form.querySelector('details[data-chooser="category"]')).map(function (i) {
      return Number(i[0]);
    });
    // createProperties: the tags' ids, the groups' ids.
    var tags = items(form.querySelector('details[data-chooser="tag"]')).map(function (i) {
      return i[0];
    });
    var groups = Array.prototype.map.call(
      form.querySelectorAll(".group-selector .d-multi-select-trigger__selected-item"),
      function (b) {
        return Number(b.dataset.value);
      }
    );
    var payload = {
      payload_url: url.value,
      content_type: Number(form.querySelector('select[name="content_type"]').value),
      secret: secret.value,
      wildcard_web_hook: wildcard,
      verify_certificate: form.querySelector('input[name="verify_certificate"]').checked,
      active: form.querySelector('input[name="active"]').checked,
      web_hook_event_type_ids: types.length ? types : [null],
      category_ids: categories.length ? categories : [null],
      tag_ids: tags.length ? tags : [null],
      group_ids: groups.length ? groups : [null],
    };
    var id = root.dataset.webHookId;
    ui.ajax(isNew ? "POST" : "PUT", isNew ? "/admin/api/web_hooks" : hookPath(id), { json: { web_hook: payload } })
      .then(function (json) {
        if (isNew) {
          go(hookPath(json.web_hook.id));
        } else {
          go("/admin/api/web_hooks");
        }
      })
      .catch(ui.failed);
  }

  // GroupSelector's groupFinder: Group.findAll({ term, ignore_automatic: false }).
  ui.multiSelect("group-selector", function (term) {
    return ui
      .ajax("GET", "/groups/search.json?ignore_automatic=false&term=" + encodeURIComponent(term || ""))
      .then(function (groups) {
        return (groups || []).map(function (g) {
          return { value: String(g.id), name: g.name };
        });
      });
  });

  document.addEventListener("click", function (event) {
    var listing = event.target.closest(".admin-contents.admin-web-hooks");
    if (!listing) {
      return;
    }
    var show = document.querySelector(".web-hook-events-listing");
    if (show && (listing.contains(show) || event.target.closest(".admin-webhooks__summary"))) {
      onShowClick(event, show);
    }
    var incoming = event.target.closest(".web-hook-events .alert-info");
    if (incoming) {
      event.preventDefault();
      location.reload();
    }
  });

  document.addEventListener("change", function (event) {
    var root = page();
    if (!root) {
      return;
    }
    var combo = event.target.closest("details.delivery-status-filters");
    if (combo) {
      var params = new URLSearchParams(location.search);
      if (combo.dataset.value) {
        params.set("status", combo.dataset.value);
      } else {
        params.delete("status");
      }
      var query = params.toString();
      window.location = location.pathname + (query ? "?" + query : "");
      return;
    }
    if (event.target.name === "wildcard") {
      syncWildcard(event.target.closest("form"));
    }
  });

  document.addEventListener(
    "submit",
    function (event) {
      var form = event.target.closest(".admin-contents.admin-web-hooks form.form-kit");
      if (form) {
        event.preventDefault();
        event.stopImmediatePropagation();
        submit(form);
      }
    },
    true
  );

  Discourse.onPage(function () {
    var root = page();
    if (!root) {
      return;
    }
    ui.strings("admin-web-hooks-strings");
    var cleanups = [];
    document.querySelectorAll(".web-hook-events .timestamp[data-time]").forEach(localTime);
    var list = document.querySelector(".admin-web_hooks__items tbody");
    if (list) {
      cleanups.push(
        ui.loadMore(function (json) {
          (json.web_hooks || []).forEach(function (hook) {
            list.appendChild(hookRow(hook, json.extras && json.extras.delivery_statuses));
          });
          return (json.web_hooks || []).length ? json.load_more_web_hooks : null;
        })
      );
    }
    var events = document.querySelector(".web-hook-events ul");
    if (events) {
      cleanups.push(
        ui.loadMore(function (json) {
          (json.web_hook_events || []).forEach(function (e) {
            events.appendChild(eventRow(e));
          });
          return (json.web_hook_events || []).length ? json.load_more_web_hook_events : null;
        })
      );
    }
    var listing = document.querySelector(".web-hook-events-listing");
    if (listing) {
      // WebhookEvents' _addIncoming: ping re-enables Ping; new events are
      // counted in the incoming notice.
      var incomingIds = [];
      cleanups.push(
        ui.subscribe("/web_hook_events/" + listing.dataset.webHookId, function (data) {
          if (data.event_type === "ping") {
            var ping = listing.querySelector(".webhook-events__ping-button");
            if (ping) {
              ping.disabled = false;
            }
          }
          if (data.type === "redelivered" || data.type === "redelivery_failed") {
            location.reload();
            return;
          }
          if (incomingIds.indexOf(data.web_hook_event_id) === -1) {
            incomingIds.push(data.web_hook_event_id);
          }
          var box = listing.querySelector(".web-hook-events");
          if (!box) {
            location.reload();
            return;
          }
          var alert = box.querySelector(".alert-info");
          if (!alert) {
            box.querySelector(".heading-container").insertAdjacentHTML(
              "afterend",
              '<a class="alert alert-info clickable" href tabindex="0"></a>'
            );
            alert = box.querySelector(".alert-info");
          }
          alert.textContent = count("admin.web_hooks.events.incoming", incomingIds.length);
        })
      );
    }
    return function () {
      cleanups.forEach(function (fn) {
        fn();
      });
    };
  });
})();
