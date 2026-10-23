// The checklist plugin's decorator (initializers/checklist.js, when
// checklist_enabled): each span.chcklst-box of a post in the stream gets
// its role, state and label, a list item's box its classes, and in a post
// the viewer may edit, the boxes outside sourced quotes are clicked (or
// toggled with Space and Enter). A toggle shows at once (optimistic,
// marked pending), is batched with the others clicked within 150ms, and
// sent to PUT /checklist/toggle with the raw and update time the page saw;
// a conflict that only changed box states is retried twice against the
// raw the server answers with. The edit's revised message on the page's
// stream carries the toggle's token (lib/optimistic-post-updates.js): the
// page that made it keeps its post as drawn instead of swapping it.
//
// What Ember's post model holds is on each post's div.cooked as
// data-checklist-post (can_edit, updated_at, version); its raw is fetched
// when a toggle first needs it, as it is in Ember, whose topic view has no
// raw either. Ember asks /posts/<id>, which isn't routed yet (ROADMAP
// 198): posts#by_number answers with the same render_post_json, raw and
// all, from the post's topic and number. Ember's model also has the cooked
// HTML, which this page doesn't keep: a post that changed since it was
// drawn, before its raw was known, is refreshed instead of compared by
// its cooked boxes. postStream.refreshPost draws the post again from the
// live stream's /live/post/<id>.
(function () {
  "use strict";

  var Discourse = window.Discourse;
  if (!Discourse || !Discourse.decorateCooked || !Discourse.once("checklist")) {
    return;
  }
  if (!Discourse.cooked.setting("checklist_enabled")) {
    return;
  }

  var SAVE_DEBOUNCE_DURATION = 150;
  var MAX_CONFLICT_RETRIES = 2;
  // Keep request batches within the toggle endpoint's contract.
  var MAX_BATCH_SIZE = 50;
  var REQUEST_TIMEOUT = 30000;

  function t(key) {
    return Discourse.cooked.t(key);
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  // lib/optimistic-post-updates.js
  var MAX_PENDING_OPTIMISTIC_UPDATES = 100;
  var OPTIMISTIC_UPDATE_TTL = 30000;
  var pendingOptimisticUpdates = new Map();

  function removeOptimisticPostUpdate(token) {
    var update = pendingOptimisticUpdates.get(token);
    if (!update) {
      return;
    }
    clearTimeout(update.timeout);
    pendingOptimisticUpdates.delete(token);
  }

  function registerOptimisticPostUpdate(token) {
    if (pendingOptimisticUpdates.has(token)) {
      removeOptimisticPostUpdate(token);
    } else if (pendingOptimisticUpdates.size >= MAX_PENDING_OPTIMISTIC_UPDATES) {
      removeOptimisticPostUpdate(pendingOptimisticUpdates.keys().next().value);
    }

    var update = {};
    pendingOptimisticUpdates.set(token, update);

    return {
      startExpiration: function () {
        if (!pendingOptimisticUpdates.has(token) || update.timeout) {
          return;
        }
        update.timeout = setTimeout(function () {
          removeOptimisticPostUpdate(token);
        }, OPTIMISTIC_UPDATE_TTL);
      },
      unregister: function () {
        removeOptimisticPostUpdate(token);
      },
    };
  }

  function consumeOptimisticPostUpdate(token) {
    if (!token || !pendingOptimisticUpdates.has(token)) {
      return false;
    }
    removeOptimisticPostUpdate(token);
    return true;
  }

  // The topic controller's "revised": a post the page changed itself is
  // not drawn again.
  document.addEventListener("htmx:sseBeforeMessage", function (event) {
    var message = event.detail;
    if (!message || message.type !== "post" || typeof message.data !== "string") {
      return;
    }
    var found = /^<template data-preserve-cooked-token="([^"]*)"><\/template>/.exec(message.data);
    if (!found) {
      return;
    }
    var token = new DOMParser().parseFromString(
      "<i data-t=\"" + found[1] + "\"></i>",
      "text/html"
    ).body.firstChild.dataset.t;
    if (consumeOptimisticPostUpdate(token)) {
      event.preventDefault();
    }
  });

  // discourse/lib/ajax with ajax-error's shape: a failure rejects with
  // { jqXHR: { status, responseJSON } }, status 0 when nothing came back.
  function ajax(method, path, data) {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["X-Requested-With"] = "XMLHttpRequest";
    headers.Accept = "application/json";
    var options = { method: method, credentials: "same-origin", headers: headers };
    if (data !== undefined) {
      headers["Content-Type"] = "application/json";
      options.body = data;
    }
    var controller = typeof AbortController === "function" ? new AbortController() : null;
    var timer;
    if (controller) {
      options.signal = controller.signal;
      timer = setTimeout(function () {
        controller.abort();
      }, REQUEST_TIMEOUT);
    }
    return fetch(basePath() + path, options).then(
      function (response) {
        clearTimeout(timer);
        return response.text().then(function (text) {
          var json = null;
          try {
            json = text ? JSON.parse(text) : null;
          } catch (e) {
            // Not JSON: the status decides.
          }
          if (!response.ok) {
            throw {
              jqXHR: { status: response.status, statusText: response.statusText, responseJSON: json },
            };
          }
          return json;
        });
      },
      function () {
        clearTimeout(timer);
        throw { jqXHR: { status: 0, statusText: "", responseJSON: null } };
      }
    );
  }

  // extractError
  function extractError(error) {
    var xhr = (error && error.jqXHR) || error || {};
    var parsed = xhr.responseJSON;
    var message;
    if (parsed) {
      if (parsed.errors && parsed.errors.length > 1) {
        message = t("multiple_errors").replace(
          "%{errors}",
          parsed.errors
            .map(function (e, i) {
              return i + 1 + ") " + e;
            })
            .join(" ")
        );
      } else if (parsed.errors && parsed.errors.length > 0) {
        message = t("generic_error_with_reason").replace("%{error}", parsed.errors[0]);
      } else if (parsed.error) {
        message = parsed.error;
      } else if (parsed.message) {
        message = parsed.message;
      } else if (parsed.failed) {
        message = parsed.failed;
      }
    }
    if (!message && xhr.status && xhr.status >= 400) {
      message = xhr.status + " " + (xhr.statusText || "");
    }
    return message || t("generic_error");
  }

  function popupAjaxError(error) {
    if (error instanceof Error && window.console) {
      console.error(error.stack);
    }
    if (Discourse.dialog) {
      Discourse.dialog.popupAjaxError(extractError(error));
    }
  }

  function timestampsAreEqual(first, second) {
    return Date.parse(first) === Date.parse(second);
  }

  function timestampIsOlder(candidate, reference) {
    return Date.parse(candidate) < Date.parse(reference);
  }

  function setCheckboxState(checkbox, checked) {
    checkbox.classList.toggle("checked", checked);
    checkbox.classList.toggle("fa-square-o", !checked);
    checkbox.classList.toggle("fa-square-check-o", checked);
    checkbox.setAttribute("aria-checked", checked.toString());
  }

  function checkboxLabel(checkbox) {
    var parts = [];
    var sibling = checkbox.nextSibling;

    while (sibling) {
      if (
        sibling.nodeType === Node.ELEMENT_NODE &&
        (sibling.classList.contains("chcklst-box") || sibling.querySelector(".chcklst-box"))
      ) {
        break;
      }
      parts.push(sibling.textContent);
      sibling = sibling.nextSibling;
    }

    var label = parts.join("").replace(/​/g, "").trim();
    return label.slice(0, 200) || t("checklist.item");
  }

  function isWhitespaceNode(node) {
    return node.nodeType === 3 && node.nodeValue.match(/^\s*$/);
  }

  function hasPrecedingContent(node) {
    var sibling = node.previousSibling;
    while (sibling) {
      if (!isWhitespaceNode(sibling)) {
        return true;
      }
      sibling = sibling.previousSibling;
    }
    return false;
  }

  function addUlClasses(boxes) {
    boxes.forEach(function (box) {
      var parent = box.parentElement;
      if (parent.nodeName === "P" && parent.parentElement.firstElementChild === parent) {
        parent = parent.parentElement;
      }

      if (
        parent.nodeName === "LI" &&
        parent.parentElement.nodeName === "UL" &&
        !hasPrecedingContent(box)
      ) {
        parent.classList.add("has-checkbox");
        box.classList.add("list-item-checkbox");
        if (!box.nextSibling) {
          // Prevent an otherwise empty list item from collapsing.
          box.insertAdjacentHTML("afterend", "&#8203;");
        }
      }
    });
  }

  function configureAccessibility(box, editable) {
    var checked = box.classList.contains("checked");
    var permanent = box.classList.contains("permanent");

    box.classList.toggle("is-interactive", editable && !permanent);
    box.setAttribute("role", "checkbox");
    box.setAttribute("aria-checked", checked.toString());
    box.setAttribute("aria-label", checkboxLabel(box));
    box.removeAttribute("aria-disabled");
    box.removeAttribute("aria-readonly");
    box.removeAttribute("tabindex");

    if (permanent) {
      box.setAttribute("aria-disabled", "true");
    } else if (editable) {
      box.setAttribute("tabindex", "0");
    } else {
      box.setAttribute("aria-readonly", "true");
    }
  }

  function checkboxKey(box, renderedIndex, checkboxCount) {
    return box.dataset.chkSrc !== undefined
      ? box.dataset.chkSrc
      : "legacy:" + renderedIndex + ":" + checkboxCount;
  }

  function checkboxTarget(box, renderedIndex, checkboxCount) {
    return {
      checkboxCount: box.dataset.chkSrc ? undefined : checkboxCount,
      checkboxSource: box.dataset.chkSrc,
      renderedIndex: renderedIndex,
    };
  }

  function checklistFingerprint(raw, checkboxSources, includesLegacy) {
    if (typeof raw !== "string") {
      return undefined;
    }

    if (includesLegacy) {
      return raw.replace(/\[(?: |x)?\]/g, "[ ]");
    }

    var sourcesByLine = new Map();
    checkboxSources.forEach(function (source) {
      var parts = source.split(":").map(Number);
      if (!sourcesByLine.has(parts[0])) {
        sourcesByLine.set(parts[0], new Set());
      }
      sourcesByLine.get(parts[0]).add(parts[1]);
    });

    var lines = raw.split(/\r\n?|\n/);
    var valid = true;
    sourcesByLine.forEach(function (indexes, lineNumber) {
      if (lines[lineNumber] === undefined) {
        valid = false;
        return;
      }

      var markerIndex = -1;
      lines[lineNumber] = lines[lineNumber].replace(/\[[ xX]?\]/g, function (marker) {
        markerIndex += 1;
        return indexes.has(markerIndex) ? "[ ]" : marker;
      });
      indexes.forEach(function (index) {
        if (index > markerIndex) {
          valid = false;
        }
      });
    });
    return valid ? lines.join("\n") : undefined;
  }

  var activeChecklistOperations = new Map();
  var uncertainChecklistBaseline =
    typeof Symbol === "function" ? Symbol("uncertain-checklist-baseline") : "__uncertainChecklistBaseline";

  function isInsideSourcedQuote(box) {
    return Boolean(
      box.closest("aside.quote[data-username], aside.quote[data-post], aside.quote[data-topic]")
    );
  }

  function cookedChecklistFingerprint(cooked) {
    if (typeof cooked !== "string") {
      return undefined;
    }
    var doc = new DOMParser().parseFromString(cooked, "text/html");
    doc.querySelectorAll(".chcklst-box:not(.permanent)").forEach(function (box) {
      if (!isInsideSourcedQuote(box)) {
        box.classList.remove("checked", "fa-square-o", "fa-square-check-o");
      }
    });
    return doc.body.innerHTML;
  }

  function checklistRendering(postModel, boxes) {
    var mutableBoxes = boxes.filter(function (box) {
      return !box.classList.contains("permanent");
    });
    var checkboxSources = mutableBoxes
      .map(function (box) {
        return box.dataset.chkSrc;
      })
      .filter(Boolean);
    var includesLegacy = mutableBoxes.some(function (box) {
      return !box.dataset.chkSrc;
    });
    var fingerprintFor = function (raw) {
      return checklistFingerprint(raw, checkboxSources, includesLegacy);
    };
    var fingerprint = fingerprintFor(postModel.raw);
    var coverage = mutableBoxes
      .map(function (box) {
        return box.dataset.chkSrc !== undefined ? box.dataset.chkSrc : "legacy";
      })
      .join("\0");
    var generation =
      fingerprint === undefined
        ? (postModel.updated_at !== undefined && postModel.updated_at !== null
            ? postModel.updated_at
            : "unknown") +
          "\0" +
          mutableBoxes
            .map(function (box) {
              return (
                (box.dataset.chkSrc !== undefined ? box.dataset.chkSrc : "legacy") +
                ":" +
                checkboxLabel(box)
              );
            })
            .join("\0")
        : fingerprint + "\0" + coverage;

    return {
      baselineUpdatedAt: postModel.updated_at,
      coverage: coverage,
      cookedFingerprint:
        fingerprint === undefined ? cookedChecklistFingerprint(postModel.cooked) : undefined,
      fingerprint: fingerprint,
      fingerprintFor: fingerprintFor,
      generation: generation,
      invalid: false,
    };
  }

  function confirmChecklistPresentation(presentation, checked) {
    var control = presentation.control;
    if (control.state && control.state.token === presentation.token) {
      control.state.confirmed = checked;
    }
  }

  function setChecklistPending(control, pending) {
    if (!control.box) {
      return;
    }
    control.box.classList.toggle("is-pending", pending);
    if (pending) {
      control.box.setAttribute("aria-busy", "true");
    } else {
      control.box.removeAttribute("aria-busy");
    }
  }

  function settleChecklistPresentation(presentation, checked) {
    var control = presentation.control;
    if (!control.state || control.state.token !== presentation.token) {
      return;
    }

    control.state = undefined;
    if (control.box) {
      setCheckboxState(control.box, checked);
    }
    setChecklistPending(control, false);
  }

  function enqueueChecklistChange(change) {
    var postId = change.postModel.id;
    if (postId === null || postId === undefined) {
      settleChecklistPresentation({ control: change.control, token: change.token }, change.confirmed);
      return;
    }

    var operation = activeChecklistOperations.get(postId);
    if (!operation) {
      operation = new ChecklistOperation(change.postModel);
      activeChecklistOperations.set(postId, operation);
    }
    operation.enqueue(change);
  }

  function newToken() {
    return typeof Symbol === "function" ? Symbol() : {};
  }

  function randomUUID() {
    if (window.crypto && crypto.randomUUID) {
      return crypto.randomUUID();
    }
    var bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    var hex = Array.prototype.map
      .call(bytes, function (b) {
        return (b + 0x100).toString(16).slice(1);
      })
      .join("");
    return (
      hex.slice(0, 8) + "-" + hex.slice(8, 12) + "-" + hex.slice(12, 16) + "-" +
      hex.slice(16, 20) + "-" + hex.slice(20)
    );
  }

  function bindChecklist(boxes, postModel) {
    var rendering = checklistRendering(postModel, boxes);
    var abortController = new AbortController();
    var controls = [];

    boxes.forEach(function (box, renderedIndex) {
      if (box.classList.contains("permanent")) {
        return;
      }

      var control = {
        box: box,
        key: checkboxKey(box, renderedIndex, boxes.length),
        rendering: rendering,
        state: undefined,
        target: checkboxTarget(box, renderedIndex, boxes.length),
      };
      controls.push(control);

      var activate = function (event) {
        if (event.type === "keydown" && event.key !== " " && event.key !== "Enter") {
          return;
        }

        event.preventDefault();
        if (event.repeat || rendering.invalid) {
          return;
        }

        var checked = !control.box.classList.contains("checked");
        var token = newToken();
        var confirmed =
          control.state && control.state.confirmed !== undefined ? control.state.confirmed : !checked;
        control.state = { confirmed: confirmed, token: token };
        setCheckboxState(control.box, checked);
        setChecklistPending(control, true);
        enqueueChecklistChange({
          checked: checked,
          confirmed: confirmed,
          control: control,
          key: control.key,
          postModel: postModel,
          rendering: rendering,
          target: control.target,
          token: token,
        });
      };

      box.addEventListener("click", activate, { signal: abortController.signal });
      box.addEventListener("keydown", activate, { signal: abortController.signal });
      var operation = activeChecklistOperations.get(postModel.id);
      if (operation) {
        operation.adopt(control);
      }
    });

    return function () {
      abortController.abort();
      controls.forEach(function (control) {
        setChecklistPending(control, false);
        control.box = undefined;
        control.state = undefined;
      });
    };
  }

  function livePresentations(presentations, except) {
    return presentations.filter(function (presentation) {
      return presentation.control.box && presentation.control !== except;
    });
  }

  function ChecklistOperation(postModel) {
    this.intents = new Map();
    this.inFlight = new Set();
    this.hydratedGenerations = new Map();
    this.postModel = postModel;
    this.revision = 0;
    this.saveTimer = undefined;
    this.saving = false;
    this.baselineRaw = postModel[uncertainChecklistBaseline] ? undefined : postModel.raw;
    this.baselineUpdatedAt = postModel.updated_at;
  }

  ChecklistOperation.prototype.adopt = function (control) {
    this.resolveGeneration(control.rendering);
    var intent = this.intents.get(control.rendering.generation + "\0" + control.key);
    if (!intent) {
      return;
    }

    var token = newToken();
    control.state = { confirmed: intent.confirmed, token: token };
    setCheckboxState(control.box, intent.checked);
    setChecklistPending(control, true);
    intent.presentations = livePresentations(intent.presentations, control);
    intent.presentations.push({ control: control, token: token });
  };

  ChecklistOperation.prototype.enqueue = function (change) {
    this.observePostModel(change.postModel);
    this.resolveGeneration(change.rendering);

    var intentKey = change.rendering.generation + "\0" + change.key;
    var previous = this.intents.get(intentKey);
    var presentations = livePresentations(previous ? previous.presentations : [], change.control);
    presentations.push({ control: change.control, token: change.token });
    var confirmed = previous ? previous.confirmed : change.confirmed;
    if (
      (!this.saving || (this.inFlight.size > 0 && !this.inFlight.has(intentKey))) &&
      change.checked === confirmed
    ) {
      this.intents.delete(intentKey);
      presentations.forEach(function (presentation) {
        settleChecklistPresentation(presentation, confirmed);
      });
      this.retireIfDrained();
      return;
    }

    this.revision += 1;
    this.intents.set(intentKey, {
      checked: change.checked,
      confirmed: confirmed,
      fingerprint: previous ? previous.fingerprint : change.rendering.fingerprint,
      presentations: presentations,
      rendering: previous ? previous.rendering : change.rendering,
      revision: this.revision,
      target: change.target,
    });

    if (!this.saving) {
      // discourseDebounce
      var self = this;
      clearTimeout(this.saveTimer);
      this.saveTimer = setTimeout(function () {
        self.save();
      }, SAVE_DEBOUNCE_DURATION);
    }
  };

  ChecklistOperation.prototype.applyBaseline = function (raw, updatedAt) {
    this.baselineRaw = raw;
    this.baselineUpdatedAt = updatedAt;
  };

  ChecklistOperation.prototype.completeBatch = function (entries) {
    var intents = this.intents;
    entries.forEach(function (entry) {
      var key = entry[0];
      var saved = entry[1];
      var current = intents.get(key);
      if (!current) {
        return;
      }

      if (current.checked === saved.checked && current.fingerprint === saved.fingerprint) {
        intents.delete(key);
        current.presentations.forEach(function (presentation) {
          settleChecklistPresentation(presentation, saved.checked);
        });
      } else {
        current.confirmed = saved.checked;
        current.presentations.forEach(function (presentation) {
          confirmChecklistPresentation(presentation, saved.checked);
        });
      }
    });
  };

  ChecklistOperation.prototype.currentEntries = function (entries) {
    var intents = this.intents;
    return entries.filter(function (entry) {
      var intent = intents.get(entry[0]);
      return intent !== undefined && intent.checked === entry[1].checked;
    });
  };

  ChecklistOperation.prototype.entriesForNextGeneration = function () {
    var all = Array.from(this.intents);
    var first = all[0][1];
    return all
      .filter(function (entry) {
        return entry[1].rendering.generation === first.rendering.generation;
      })
      .slice(0, MAX_BATCH_SIZE);
  };

  ChecklistOperation.prototype.failBatch = function (entries) {
    var intents = this.intents;
    entries.forEach(function (entry) {
      var current = intents.get(entry[0]);
      if (!current || current.revision !== entry[1].revision) {
        return;
      }

      intents.delete(entry[0]);
      current.presentations.forEach(function (presentation) {
        settleChecklistPresentation(presentation, current.confirmed);
      });
    });
  };

  ChecklistOperation.prototype.failGeneration = function (generation, invalidate) {
    var entries = Array.from(this.intents).filter(function (entry) {
      return entry[1].rendering.generation === generation;
    });
    if (invalidate) {
      entries.forEach(function (entry) {
        var intent = entry[1];
        intent.rendering.invalid = true;
        intent.presentations.forEach(function (presentation) {
          presentation.control.rendering.invalid = true;
          if (presentation.control.box) {
            configureAccessibility(presentation.control.box, false);
          }
        });
      });
    }
    this.failBatch(entries);
  };

  ChecklistOperation.prototype.freshestBaseline = function (raw, updatedAt) {
    var model = this.postModel;
    if (
      model.raw !== null &&
      model.raw !== undefined &&
      (timestampIsOlder(updatedAt, model.updated_at) ||
        (timestampsAreEqual(updatedAt, model.updated_at) && raw !== model.raw))
    ) {
      return { raw: model.raw, updatedAt: model.updated_at };
    }
    return { raw: raw, updatedAt: updatedAt };
  };

  // #hydrate: resolves true when the entries have a baseline to send.
  ChecklistOperation.prototype.hydrate = function (entries) {
    var self = this;
    if (
      this.baselineRaw !== null &&
      this.baselineRaw !== undefined &&
      entries.every(function (entry) {
        return entry[1].fingerprint !== undefined;
      })
    ) {
      return Promise.resolve(true);
    }

    var rendering = entries[0][1].rendering;
    return ajax(
      "GET",
      "/posts/by_number/" + this.postModel.topic_id + "/" + this.postModel.post_number + ".json"
    ).then(
      function (post) {
        var baseline = self.postModel[uncertainChecklistBaseline]
          ? { raw: post.raw, updatedAt: post.updated_at }
          : self.freshestBaseline(post.raw, post.updated_at);
        var fingerprint = rendering.fingerprintFor(baseline.raw);
        var canUseBaseline = entries.every(function (entry) {
          var intent = entry[1];
          return (
            timestampsAreEqual(intent.rendering.baselineUpdatedAt, baseline.updatedAt) ||
            (intent.fingerprint !== undefined &&
              intent.rendering.fingerprintFor(baseline.raw) === intent.fingerprint) ||
            (intent.rendering.cookedFingerprint !== undefined &&
              intent.rendering.cookedFingerprint === cookedChecklistFingerprint(post.cooked))
          );
        });
        if (!canUseBaseline || fingerprint === undefined) {
          self.failGeneration(rendering.generation, true);
          self.refreshPost(true);
          return false;
        }

        var oldGeneration = rendering.generation;
        var resolved = {
          fingerprint: fingerprint,
          generation: fingerprint + "\0" + rendering.coverage,
        };
        self.hydratedGenerations.set(oldGeneration, resolved);
        Array.from(self.intents).forEach(function (pair) {
          var key = pair[0];
          var intent = pair[1];
          if (
            intent.rendering.generation !== oldGeneration &&
            key.indexOf(oldGeneration + "\0") !== 0
          ) {
            return;
          }
          self.resolveGeneration(intent.rendering);
          intent.fingerprint = fingerprint;
          intent.presentations.forEach(function (presentation) {
            self.resolveGeneration(presentation.control.rendering);
          });
          var newKey = resolved.generation + key.slice(oldGeneration.length);
          var existing = self.intents.get(newKey);
          var merged = intent;
          if (existing && existing !== intent) {
            var latest = existing.revision > intent.revision ? existing : intent;
            merged = Object.assign({}, latest, {
              presentations: existing.presentations
                .concat(intent.presentations)
                .filter(function (presentation) {
                  return (
                    presentation.control.box &&
                    presentation.control.state &&
                    presentation.control.state.token === presentation.token
                  );
                }),
            });
          }
          self.intents.delete(key);
          self.intents.set(newKey, merged);
          entries.forEach(function (entry) {
            if (entry[0] === key) {
              entry[0] = newKey;
              entry[1].fingerprint = fingerprint;
              if (existing && existing !== intent) {
                entry[1] = merged;
              }
            }
          });
        });
        self.applyBaseline(baseline.raw, baseline.updatedAt);
        delete self.postModel[uncertainChecklistBaseline];
        if (self.postModel.raw === null || self.postModel.raw === undefined) {
          self.postModel.raw = baseline.raw;
        }
        return true;
      },
      function (error) {
        self.postModel[uncertainChecklistBaseline] = true;
        self.failGeneration(rendering.generation);
        popupAjaxError(error);
        return false;
      }
    );
  };

  ChecklistOperation.prototype.observePostModel = function (postModel) {
    if (
      this.postModel.raw === null ||
      this.postModel.raw === undefined ||
      timestampIsOlder(this.postModel.updated_at, postModel.updated_at) ||
      (Number.isFinite(postModel.version) &&
        Number.isFinite(this.postModel.version) &&
        postModel.version > this.postModel.version)
    ) {
      this.postModel = postModel;
    }
  };

  // postStream.refreshPost: the post drawn again from the server.
  ChecklistOperation.prototype.refreshPost = function (reportError) {
    var path = basePath() + "/live/post/" + this.postModel.id;
    var refresh;
    if (window.htmx) {
      refresh = window.htmx.ajax("GET", path, { swap: "none" });
    }
    if (refresh && refresh.catch) {
      refresh.catch(reportError ? popupAjaxError : function () {});
    }
  };

  ChecklistOperation.prototype.resolveGeneration = function (rendering) {
    var resolved = this.hydratedGenerations.get(rendering.generation);
    if (resolved) {
      Object.assign(rendering, resolved);
    }
  };

  ChecklistOperation.prototype.retireIfDrained = function () {
    if (this.saving || this.intents.size > 0) {
      return;
    }

    if (this.saveTimer) {
      clearTimeout(this.saveTimer);
      this.saveTimer = undefined;
    }
    if (activeChecklistOperations.get(this.postModel.id) === this) {
      activeChecklistOperations.delete(this.postModel.id);
    }
  };

  // #save: batches of one generation's intents, each sent (retried on a
  // conflict over box states only) until none are left. The loops of
  // Ember's async method as promise steps: nextBatch is the outer loop,
  // attempt the inner one.
  ChecklistOperation.prototype.save = function () {
    var self = this;
    this.saveTimer = undefined;
    if (this.saving) {
      return Promise.resolve();
    }

    this.saving = true;

    function nextBatch() {
      if (self.intents.size === 0) {
        return Promise.resolve();
      }
      return attempt(self.entriesForNextGeneration(), 0);
    }

    function attempt(entries, retries) {
      entries = self.currentEntries(entries);
      if (entries.length === 0) {
        return nextBatch();
      }
      return self.hydrate(entries).then(function (hydrated) {
        if (!hydrated) {
          return nextBatch();
        }
        entries = self.currentEntries(entries);
        if (entries.length === 0) {
          return nextBatch();
        }
        return send(entries, retries);
      });
    }

    function send(entries, retries) {
      var model = self.postModel;
      if (
        model.raw !== null &&
        model.raw !== undefined &&
        timestampIsOlder(self.baselineUpdatedAt, model.updated_at)
      ) {
        self.applyBaseline(model.raw, model.updated_at);
      }

      var rendering = entries[0][1].rendering;
      var fingerprint = entries[0][1].fingerprint;
      var baselineFingerprint = rendering.fingerprintFor(self.baselineRaw);
      var modelFingerprint = rendering.fingerprintFor(model.raw);
      if (
        fingerprint === undefined ||
        baselineFingerprint === undefined ||
        fingerprint !== baselineFingerprint ||
        (model.raw !== null && model.raw !== undefined && fingerprint !== modelFingerprint)
      ) {
        self.failGeneration(rendering.generation, true);
        self.refreshPost(true);
        return nextBatch();
      }

      var mutationId = randomUUID();
      var optimistic = registerOptimisticPostUpdate(mutationId);
      var data = {
        post_id: model.id,
        toggles: entries.map(function (entry) {
          var intent = entry[1];
          var toggle = {
            checkbox_index: intent.target.renderedIndex,
            checkbox_source: intent.target.checkboxSource,
            checked: intent.checked,
          };
          if (!intent.target.checkboxSource) {
            toggle.checkbox_count = intent.target.checkboxCount;
          }
          return toggle;
        }),
        expected_raw: self.baselineRaw,
        expected_updated_at: self.baselineUpdatedAt,
        mutation_id: mutationId,
      };

      self.inFlight = new Set(
        entries.map(function (entry) {
          return entry[0];
        })
      );
      return ajax("PUT", "/checklist/toggle", JSON.stringify(data)).then(
        function (response) {
          var next = received(response, entries, retries, rendering, fingerprint, optimistic);
          self.inFlight.clear();
          return next();
        },
        function (error) {
          var next = failed(error, entries, retries, rendering, fingerprint, optimistic);
          self.inFlight.clear();
          return next();
        }
      );
    }

    // What follows a 200: the next step to take.
    function received(response, entries, retries, rendering, fingerprint, optimistic) {
      var model = self.postModel;
      var stale =
        (Number.isFinite(response.version) &&
          Number.isFinite(model.version) &&
          response.version < model.version) ||
        timestampIsOlder(response.updated_at, model.updated_at);

      if (response.revised) {
        optimistic.startExpiration();
      } else {
        optimistic.unregister();
      }

      if (stale) {
        if (response.raw === model.raw) {
          self.completeBatch(entries);
          self.applyBaseline(model.raw, model.updated_at);
          return nextBatch;
        }

        var baseline = self.freshestBaseline(response.raw, response.updated_at);
        var canRetry =
          retries < MAX_CONFLICT_RETRIES && rendering.fingerprintFor(baseline.raw) === fingerprint;
        self.applyBaseline(baseline.raw, baseline.updatedAt);
        if (canRetry) {
          return function () {
            return attempt(entries, retries + 1);
          };
        }

        self.failBatch(entries);
        self.refreshPost();
        return nextBatch;
      }

      if (rendering.fingerprintFor(model.raw) !== fingerprint) {
        self.applyBaseline(model.raw, model.updated_at);
        self.failGeneration(rendering.generation, true);
        self.refreshPost();
        return nextBatch;
      }

      model.last_editor_id = response.last_editor_id;
      model.updated_at = response.updated_at;
      model.version = response.version;
      model.raw = response.raw;
      self.applyBaseline(response.raw, response.updated_at);
      self.completeBatch(entries);
      model.cooked = response.cooked;
      return nextBatch;
    }

    // What follows a failed request.
    function failed(error, entries, retries, rendering, fingerprint, optimistic) {
      optimistic.unregister();
      var xhr = error && error.jqXHR;
      var conflict = xhr && xhr.responseJSON;
      var baseline =
        conflict && conflict.raw && conflict.updated_at
          ? self.freshestBaseline(conflict.raw, conflict.updated_at)
          : undefined;
      var canRetry =
        xhr &&
        xhr.status === 409 &&
        conflict.retryable &&
        baseline &&
        retries < MAX_CONFLICT_RETRIES &&
        rendering.fingerprintFor(baseline.raw) === fingerprint;

      if (canRetry) {
        self.applyBaseline(baseline.raw, baseline.updatedAt);
        return function () {
          return attempt(entries, retries + 1);
        };
      }

      var structuralConflict =
        xhr && xhr.status === 409 && baseline && rendering.fingerprintFor(baseline.raw) !== fingerprint;
      if (structuralConflict) {
        self.failGeneration(rendering.generation, true);
      } else {
        self.failBatch(entries);
      }
      if (baseline) {
        self.applyBaseline(baseline.raw, baseline.updatedAt);
      } else if (!xhr || xhr.status === 0) {
        self.baselineRaw = undefined;
        self.postModel[uncertainChecklistBaseline] = true;
      } else if (self.postModel.raw !== null && self.postModel.raw !== undefined) {
        self.applyBaseline(self.postModel.raw, self.postModel.updated_at);
      }
      self.refreshPost();
      popupAjaxError(error);
      return nextBatch;
    }

    return nextBatch()
      .catch(function (error) {
        self.failBatch(Array.from(self.intents));
        popupAjaxError(error);
      })
      .then(function () {
        self.saving = false;
        self.retireIfDrained();
      });
  };

  // The post model of a stream post's cooked element: one object per post
  // that lives as long as the page, as Ember's does, its fields updated
  // when the post is drawn again with a newer one.
  var postModels = new Map();

  function postModelFor(elem) {
    var holder = elem.closest("[data-checklist-post]");
    var article = elem.closest("article[data-post-id]");
    if (!holder || !article) {
      return undefined;
    }
    var drawn;
    try {
      drawn = JSON.parse(holder.dataset.checklistPost);
    } catch (e) {
      return undefined;
    }
    var id = Number(article.dataset.postId);
    var model = postModels.get(id);
    if (!model) {
      model = {
        id: id,
        topic_id: drawn.topic_id,
        post_number: drawn.post_number,
        raw: undefined,
        cooked: undefined,
      };
      postModels.set(id, model);
    }
    model.can_edit = drawn.can_edit;
    if (model.updated_at === undefined || timestampIsOlder(model.updated_at, drawn.updated_at)) {
      model.updated_at = drawn.updated_at;
      model.version = drawn.version;
      model.raw = undefined;
      model.cooked = undefined;
    }
    return model;
  }

  // The decorators' teardown (decorateCookedElement's return value) once
  // their element leaves the page.
  var teardowns = [];
  new MutationObserver(function () {
    teardowns = teardowns.filter(function (entry) {
      if (entry.elem.isConnected) {
        return true;
      }
      entry.teardown();
      return false;
    });
  }).observe(document.documentElement, { childList: true, subtree: true });

  // checklistSyntax, on posts in the stream only (onlyStream).
  function checklistSyntax(elem) {
    var postModel = postModelFor(elem);
    if (!postModel) {
      return;
    }
    var boxes = Array.prototype.slice.call(elem.getElementsByClassName("chcklst-box"));
    addUlClasses(boxes);

    var editable = postModel.can_edit === true;
    var interactiveBoxes = boxes.filter(function (box) {
      return !isInsideSourcedQuote(box);
    });
    var interactiveBoxSet = new Set(interactiveBoxes);

    boxes.forEach(function (box) {
      configureAccessibility(box, editable && interactiveBoxSet.has(box));
    });

    if (editable && interactiveBoxes.length > 0) {
      teardowns.push({ elem: elem, teardown: bindChecklist(interactiveBoxes, postModel) });
    }
  }

  Discourse.decorateCooked("checklist", checklistSyntax);
})();
