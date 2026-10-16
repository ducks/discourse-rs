// The admin backups pages in the browser, as the admin.backups route and
// its controllers act: the header's actions (the start backup modal,
// cancel, rollback, the read-only and archive mode toggles), a backup's
// download link email, its restore and delete from the row's menu, the
// uploader's chunked upload (UppyBackupUploader's resumable chunks for
// the local store), and the logs as the message bus delivers them. A
// change the page shows otherwise (the actions' labels, the files) loads
// it again.
(function () {
  "use strict";

  if (!Discourse.once("admin-backups")) {
    return;
  }

  var ui = Discourse.adminUi;
  var t = ui.t;

  // Uppy's chunk size for the local store's resumable uploads.
  var CHUNK_SIZE = 5 * 1024 * 1024;

  function page() {
    return document.querySelector(".admin-contents.backups");
  }

  function go(path) {
    window.location = ui.base() + path;
  }

  function clientId() {
    var key = "admin-backups-client-id";
    var id = sessionStorage.getItem(key);
    if (!id) {
      id = Array.from(crypto.getRandomValues(new Uint8Array(16)), function (b) {
        return b.toString(16).padStart(2, "0");
      }).join("");
      try {
        sessionStorage.setItem(key, id);
      } catch (_) {
        // Storage blocked: a new id each time.
      }
    }
    return id;
  }

  // StartBackup: include uploads, then Backup.start and the logs.
  function showStartBackup() {
    var root = page();
    var s3Warning = root.dataset.s3Warning === "true";
    var canManageUploads = root.dataset.canManageUploads !== "false";
    var include;
    Discourse.modal.show({
      className: "start-backup-modal",
      title: t("admin.backups.operations.backup.confirm"),
      body: function (body) {
        if (s3Warning) {
          var warning = document.createElement("div");
          warning.className = "";
          warning.innerHTML = t("admin.backups.operations.backup.s3_upload_warning");
          body.appendChild(warning);
        }
        if (canManageUploads) {
          var label = document.createElement("label");
          label.className = "checkbox-label";
          include = document.createElement("input");
          include.type = "checkbox";
          include.checked = true;
          label.appendChild(include);
          label.appendChild(document.createTextNode(" " + t("admin.backups.operations.backup.include_uploads")));
          body.appendChild(label);
        }
      },
      footer: function (footer, close) {
        var yes = document.createElement("button");
        yes.className = "btn btn-primary";
        yes.type = "button";
        yes.innerHTML = '<span class="d-button-label"></span>';
        yes.firstChild.textContent = t("yes_value");
        yes.addEventListener("click", function () {
          var withUploads = include ? include.checked : true;
          close();
          ui.ajax("POST", "/admin/backups", { with_uploads: withUploads, client_id: clientId() })
            .then(function (result) {
              if (result && result.success === false) {
                Discourse.dialog.alert(result.message);
              } else {
                go("/admin/backups/logs");
              }
            })
            .catch(ui.failed);
        });
        var cancel = document.createElement("button");
        cancel.className = "btn btn-flat";
        cancel.type = "button";
        cancel.innerHTML = '<span class="d-button-label"></span>';
        cancel.firstChild.textContent = t("cancel");
        cancel.addEventListener("click", close);
        footer.appendChild(yes);
        footer.appendChild(cancel);
      },
    });
  }

  function confirm(key, then) {
    Discourse.dialog.yesNoConfirm({ message: t(key), didConfirm: then });
  }

  function reload() {
    location.reload();
  }

  function onHeaderAction(button) {
    if (button.classList.contains("admin-backups__start")) {
      showStartBackup();
    } else if (button.classList.contains("admin-backups__cancel")) {
      confirm("admin.backups.operations.cancel.confirm", function () {
        ui.ajax("DELETE", "/admin/backups/cancel.json").then(reload).catch(ui.failed);
      });
    } else if (button.classList.contains("admin-backups__rollback")) {
      confirm("admin.backups.operations.rollback.confirm", function () {
        ui.ajax("POST", "/admin/backups/rollback.json")
          .then(function (result) {
            if (result && result.success === false) {
              Discourse.dialog.alert(result.message);
            } else {
              go("/");
            }
          })
          .catch(ui.failed);
      });
    } else if (button.classList.contains("admin-backups__toggle-read-only")) {
      var readOnly = page().dataset.readOnly === "true";
      var toggle = function () {
        ui.ajax("PUT", "/admin/backups/readonly", { enable: !readOnly }).then(reload).catch(ui.failed);
      };
      if (readOnly) {
        toggle();
      } else {
        confirm("admin.backups.read_only.enable.confirm", toggle);
      }
    } else if (button.classList.contains("admin-backups__toggle-site-archived")) {
      var archived = page().dataset.siteArchived === "true";
      var set = function () {
        ui.ajax("PUT", "/admin/site_settings/site_archived", { site_archived: !archived })
          .then(reload)
          .catch(ui.failed);
      };
      if (archived) {
        set();
      } else {
        confirm("admin.backups.site_archived.enable.confirm", set);
      }
    }
  }

  function filenameOf(el) {
    var row = el.closest("[data-backup-filename]");
    return row ? row.dataset.backupFilename : null;
  }

  // The row's menu: restore and delete, the menu's content knowing its
  // row through the trigger.
  ui.menus(function (trigger, content, menu) {
    var filename = filenameOf(trigger);
    content.addEventListener("click", function (event) {
      var button = event.target.closest("button");
      if (!button || button.disabled || !filename) {
        return;
      }
      if (menu && menu.close) {
        menu.close();
      }
      var path = "/admin/backups/" + encodeURIComponent(filename);
      if (button.classList.contains("backup-item-row__restore")) {
        confirm("admin.backups.operations.restore.confirm", function () {
          ui.ajax("POST", path + "/restore", { client_id: clientId() })
            .then(function (result) {
              if (result && result.success === false) {
                Discourse.dialog.alert(result.message);
              } else {
                go("/admin/backups/logs");
              }
            })
            .catch(ui.failed);
        });
      } else if (button.classList.contains("backup-item-row__delete")) {
        confirm("admin.backups.operations.destroy.confirm", function () {
          ui.ajax("DELETE", path)
            .then(function () {
              var row = trigger.closest("tr");
              var body = row.parentElement;
              row.remove();
              if (!body.querySelector("tr")) {
                reload();
              }
            })
            .catch(ui.failed);
        });
      }
    });
  });

  // The resumable chunks Rails' check_backup_chunk and upload_backup_chunk
  // take, one after another.
  function upload(input) {
    var file = input.files && input.files[0];
    if (!file) {
      return;
    }
    var label = input.closest("label");
    var text = label.firstChild;
    var original = text.textContent;
    var total = Math.max(1, Math.floor(file.size / CHUNK_SIZE));
    var identifier = file.size + "-" + file.name.replace(/[^0-9a-zA-Z_-]/g, "");
    input.disabled = true;
    label.setAttribute("disabled", "");
    function progress(done) {
      text.textContent = t("admin.backups.upload.uploading_progress", {
        progress: Math.round((done / file.size) * 100),
      });
    }
    function chunk(n) {
      var start = (n - 1) * CHUNK_SIZE;
      var end = n === total ? file.size : start + CHUNK_SIZE;
      var form = new FormData();
      form.append("resumableChunkNumber", n);
      form.append("resumableChunkSize", CHUNK_SIZE);
      form.append("resumableCurrentChunkSize", end - start);
      form.append("resumableTotalSize", file.size);
      form.append("resumableType", file.type);
      form.append("resumableIdentifier", identifier);
      form.append("resumableFilename", file.name);
      form.append("resumableRelativePath", file.name);
      form.append("resumableTotalChunks", total);
      form.append("file", file.slice(start, end), file.name);
      return ui.ajax("POST", "/admin/backups/upload", form).then(function () {
        progress(end);
        return n < total ? chunk(n + 1) : null;
      });
    }
    progress(0);
    chunk(1)
      .then(function () {
        Discourse.dialog.alert(t("admin.backups.upload.success", { filename: file.name }));
      })
      .catch(function (e) {
        Discourse.dialog.alert(
          t("admin.backups.upload.error", { filename: file.name, message: e.message })
        );
      })
      .then(function () {
        text.textContent = original;
        input.disabled = false;
        input.value = "";
        label.removeAttribute("disabled");
      });
  }

  document.addEventListener("click", function (event) {
    if (!page()) {
      return;
    }
    var header = event.target.closest(".d-page-header__actions .d-page-action-button");
    if (header && !header.disabled && !header.dataset.pageAction) {
      onHeaderAction(header);
      return;
    }
    var download = event.target.closest(".backup-item-row__download");
    if (download) {
      ui.ajax("PUT", "/admin/backups/" + encodeURIComponent(filenameOf(download)))
        .then(function () {
          Discourse.dialog.alert(t("admin.backups.operations.download.alert"));
        })
        .catch(ui.failed);
    }
  });

  document.addEventListener("change", function (event) {
    var input = event.target.closest(".admin-backups-upload .hidden-upload-field");
    if (input && page()) {
      upload(input);
    }
  });

  // AdminBackupsRoute#onMessage and the logs controller: the lines as they
  // come, [STARTED] clearing them, [FAILED] alerting, [SUCCESS] ending
  // the operation (a restore going to the home page).
  function followLogs() {
    var logs = document.querySelector(".admin-backups-logs");
    var root = page();
    return ui.subscribe("/admin/backups/logs", function (log) {
      if (log.message === "[STARTED]") {
        if (logs) {
          logs.querySelector("pre, p").outerHTML = "<pre></pre>";
          setRunning(logs, true);
        }
      } else if (log.message === "[FAILED]") {
        if (logs) {
          setRunning(logs, false);
        }
        Discourse.dialog.alert(t("admin.backups.operations.failed", { operation: log.operation }));
      } else if (log.message === "[SUCCESS]") {
        if (log.operation === "restore") {
          go("/");
        } else if (logs) {
          setRunning(logs, false);
        } else if (root) {
          reload();
        }
      } else if (logs && log.message) {
        var pre = logs.querySelector("pre");
        if (!pre) {
          logs.querySelector("p").outerHTML = "<pre></pre>";
          pre = logs.querySelector("pre");
        }
        pre.textContent += "[" + log.timestamp + "] " + log.message + "\n";
        logs.scrollTop = logs.scrollHeight;
      }
    });
  }

  function setRunning(logs, running) {
    var spinner = logs.querySelector(".loading-container");
    spinner.classList.toggle("visible", running);
    if (running) {
      spinner.setAttribute("data-loading", "true");
      spinner.innerHTML = '<div class="spinner small"></div>';
    } else {
      spinner.removeAttribute("data-loading");
      spinner.innerHTML = "";
    }
  }

  Discourse.onPage(function () {
    if (!page()) {
      return;
    }
    ui.strings("admin-backups-strings");
    var logs = document.querySelector(".admin-backups-logs");
    if (logs) {
      logs.scrollTop = logs.scrollHeight;
    }
    return followLogs();
  });
})();
