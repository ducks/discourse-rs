// The tag groups pages (src/tags/groups_view.rs): the sidebar's New
// group button, and TagGroupsForm's save and delete. The choosers are
// prefs-choosers.js'; the form reads their data-items. Saving posts the
// tag_group hash as JSON, so the permission types stay integers
// (TagGroup.resolve_permissions resolves a string by name): a new group
// then goes to the index, an edited one stays; a deleted one goes to the
// index.
(function () {
  "use strict";

  if (!Discourse.once("tag-groups")) {
    return;
  }

  var FULL = 1;
  var READONLY = 3;
  var EVERYONE = "0";

  function meta(name) {
    return (document.querySelector('meta[name="' + name + '"]') || {}).content || "";
  }

  function basePath() {
    return meta("discourse-base-uri");
  }

  function strings(form) {
    try {
      return JSON.parse(form.dataset.strings || "{}");
    } catch (e) {
      return {};
    }
  }

  function items(chooser) {
    try {
      return JSON.parse((chooser && chooser.dataset.items) || "[]");
    } catch (e) {
      return [];
    }
  }

  // TagGroupsForm#serializeTag: a known tag by id and name, a new one by
  // name.
  function serializeTag(item) {
    return /^\d+$/.test(String(item[0])) ? { id: parseInt(item[0], 10), name: item[1] } : { name: item[1] };
  }

  function alert(message) {
    Discourse.dialog.alert(message);
  }

  function request(method, url, body) {
    var headers = {
      "X-CSRF-Token": meta("csrf-token"),
      "X-Requested-With": "XMLHttpRequest",
      Accept: "application/json",
    };
    if (body) {
      headers["Content-Type"] = "application/json";
    }
    return fetch(url, { method: method, headers: headers, body: body, credentials: "same-origin" }).then(function (response) {
      return response
        .json()
        .catch(function () {
          return {};
        })
        .then(function (json) {
          if (!response.ok) {
            var error = (json.errors && json.errors.join("\n")) || json.error || response.statusText;
            throw new Error(error);
          }
          return json;
        });
    });
  }

  function save(form) {
    var s = strings(form);
    var name = form.querySelector('input[name="name"]').value;
    var tags = items(form.querySelector('details[data-field="tags"]')).map(serializeTag);
    var parent = items(form.querySelector('details[data-field="parent_tag"]')).map(serializeTag);
    var checked = form.querySelector('input[name="tag-permissions-choice"]:checked');
    var permissionName = checked ? checked.value : "public";
    if (!name.trim()) {
      alert(s.empty_name);
      return;
    }
    if (!tags.length) {
      alert(s.no_tags);
      return;
    }
    // setPermissionsGroups: the chosen groups with FULL.
    var permissions = {};
    items(form.querySelector("details[data-group-ids]")).forEach(function (i) {
      permissions[i[0]] = FULL;
    });
    if (permissionName === "public") {
      permissions = {};
      permissions[EVERYONE] = FULL;
    } else if (permissionName === "visible") {
      permissions[EVERYONE] = READONLY;
    } else {
      delete permissions[EVERYONE];
      if (!Object.keys(permissions).length) {
        alert(s.no_groups);
        return;
      }
    }
    var attrs = {
      name: name,
      tags: tags,
      parent_tag: parent,
      one_per_topic: form.querySelector('input[name="onepertopic"]').checked,
      permissions: permissions,
    };
    var id = form.dataset.tagGroupId;
    var body = JSON.stringify({ tag_group: attrs });
    var button = form.querySelector("[data-tag-group-save]");
    button.disabled = true;
    request(id ? "PUT" : "POST", basePath() + "/tag_groups" + (id ? "/" + id : "") + ".json", body)
      .then(function () {
        if (!id) {
          window.location.href = basePath() + "/tag_groups";
        }
      })
      .catch(function (e) {
        Discourse.dialog.popupAjaxError(e.message);
      })
      .then(function () {
        button.disabled = false;
      });
  }

  function destroy(form) {
    var s = strings(form);
    Discourse.dialog.yesNoConfirm({
      message: s.confirm_delete,
      didConfirm: function () {
        request("DELETE", basePath() + "/tag_groups/" + form.dataset.tagGroupId + ".json")
          .then(function () {
            window.location.href = basePath() + "/tag_groups";
          })
          .catch(function (e) {
            Discourse.dialog.popupAjaxError(e.message);
          });
      },
    });
  }

  document.addEventListener("click", function (event) {
    var add = event.target.closest("[data-tag-groups-new]");
    if (add) {
      window.location.href = add.dataset.tagGroupsNew;
      return;
    }
    var form = event.target.closest("form.tag-group-form");
    if (!form) {
      return;
    }
    if (event.target.closest("[data-tag-group-save]")) {
      event.preventDefault();
      save(form);
    } else if (event.target.closest("[data-tag-group-delete]")) {
      event.preventDefault();
      destroy(form);
    }
  });

  // Both GroupChoosers show selectedGroupIds: a change to one is the
  // other's.
  var syncing = false;
  new MutationObserver(function (records) {
    if (syncing) {
      return;
    }
    records.forEach(function (record) {
      var chooser = record.target;
      var form = chooser.closest && chooser.closest("form.tag-group-form");
      if (!form) {
        return;
      }
      syncing = true;
      form.querySelectorAll("details[data-group-ids]").forEach(function (other) {
        if (other !== chooser && other.dataset.items !== chooser.dataset.items) {
          other.dataset.items = chooser.dataset.items;
          var summary = other.querySelector("summary");
          var source = chooser.querySelector("summary");
          summary.dataset.name = source.dataset.name;
          summary.dataset.value = source.dataset.value;
          other.querySelector(".formatted-selection").textContent = chooser.querySelector(".formatted-selection").textContent;
          other.classList.toggle("has-selection", chooser.classList.contains("has-selection"));
        }
      });
      syncing = false;
    });
  }).observe(document.body, { attributes: true, attributeFilter: ["data-items"], subtree: true });
})();
