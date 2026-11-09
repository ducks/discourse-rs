// components/modal/change-reply-to.gjs: the edit composer's reply target
// picker. The topic's fullscreen timeline (GET /live/timeline/:topic_id,
// driven by topic.js' Discourse.topicTimeline) picks a post as it is
// scrolled; one earlier than the post edited can be set as its target,
// and a current target removed.
//
//   Discourse.changeReplyTo.show({ topicId, editingPostNumber,
//                                  currentPostNumber, onSelect, t,
//                                  timelineScript })
//       onSelect(post) gets the post picked (the posts#show JSON), or null
//       to remove the target; t(key, vars) the composer's strings;
//       timelineScript topic.js' url, loaded off a topic page.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.changeReplyTo) {
    return;
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function fetchJson(path) {
    return fetch(basePath() + path, {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    }).then(function (r) {
      return r.json().then(
        function (json) {
          if (!r.ok) {
            throw json;
          }
          return json;
        },
        function () {
          throw new Error(r.status + " " + r.statusText);
        }
      );
    });
  }

  // lib/ajax-error extractError
  function extractError(e) {
    if (e && Array.isArray(e.errors) && e.errors.length) {
      return e.errors.join("\n");
    }
    return (e && e.message) || String(e);
  }

  var loadingTimeline = null;
  function loadTimeline(src) {
    if (Discourse.topicTimeline) {
      return Promise.resolve(Discourse.topicTimeline);
    }
    if (!loadingTimeline) {
      loadingTimeline = new Promise(function (resolve, reject) {
        var script = document.createElement("script");
        script.src = src;
        script.onload = function () {
          resolve(Discourse.topicTimeline);
        };
        script.onerror = reject;
        document.head.appendChild(script);
      });
    }
    return loadingTimeline;
  }

  function button(className, label) {
    var b = document.createElement("button");
    b.className = "btn btn-text" + (className ? " " + className : "");
    b.type = "button";
    b.innerHTML = '<span class="d-button-label"></span>';
    b.querySelector(".d-button-label").textContent = label;
    return b;
  }

  function show(opts) {
    var t = opts.t;
    var selectedPost = null;
    var submitButton = null;
    var hint = null;
    var modal = null;
    var cleanup = null;

    function setFlash(text) {
      var flash = modal && modal.querySelector("#modal-alert");
      if (!text) {
        if (flash) {
          flash.remove();
        }
        return;
      }
      if (!flash) {
        flash = document.createElement("div");
        flash.id = "modal-alert";
        flash.setAttribute("role", "alert");
        flash.className = "alert";
        var body = modal.querySelector(".d-modal__body");
        body.parentNode.insertBefore(flash, body);
      }
      flash.textContent = text;
    }

    // canSubmit: a post earlier than the one edited
    function canSubmit() {
      return !!selectedPost && selectedPost.post_number < opts.editingPostNumber;
    }

    function drawSelection() {
      if (selectedPost) {
        hint.className = "change-reply-to-modal__selection";
        hint.textContent = t("composer.change_reply_to.current_selection", {
          post_number: selectedPost.post_number,
          username: selectedPost.username,
        });
      } else {
        hint.className = "change-reply-to-modal__hint";
        hint.textContent = t("composer.change_reply_to.hint");
      }
      if (submitButton) {
        submitButton.disabled = !canSubmit();
      }
    }

    // captureSelectionForIndex: the post at a timeline index (1-based).
    function capture(stream, timelineIndex) {
      var postId = stream[timelineIndex - 1];
      if (!timelineIndex || !postId) {
        return;
      }
      fetchJson("/posts/" + postId + ".json")
        .then(function (post) {
          if (!post) {
            return;
          }
          if (post.post_number >= opts.editingPostNumber) {
            setFlash(t("composer.change_reply_to.invalid_target"));
            selectedPost = null;
          } else {
            setFlash(null);
            selectedPost = post;
          }
          drawSelection();
        })
        .catch(function (e) {
          setFlash(extractError(e));
        });
    }

    var close = Discourse.modal.show({
      className: "change-reply-to-modal",
      title: t("composer.change_reply_to.title"),
      body: function (body) {
        modal = body.closest(".d-modal");
        hint = document.createElement("p");
        body.appendChild(hint);
        drawSelection();
        var current = opts.currentPostNumber
          ? fetchJson("/posts/by_number/" + opts.topicId + "/" + opts.currentPostNumber).catch(function () {
              return null;
            })
          : Promise.resolve(null);
        Promise.all([
          fetch(basePath() + "/live/timeline/" + opts.topicId, { credentials: "same-origin" }).then(function (r) {
            if (!r.ok) {
              throw new Error("/live/timeline " + r.status);
            }
            return r.text();
          }),
          current,
          loadTimeline(opts.timelineScript),
        ])
          .then(function (results) {
            if (!body.isConnected) {
              return;
            }
            var template = document.createElement("template");
            template.innerHTML = results[0].trim();
            var container = template.content.firstElementChild;
            body.appendChild(container);
            container.classList.add("show");
            var stream = JSON.parse(container.dataset.stream);
            // initialEnteredIndex: the current target's place in the stream
            var currentPost = results[1];
            var entered = currentPost ? stream.indexOf(currentPost.id) + 1 : 0;
            cleanup = results[2].setup(container, {
              enteredIndex: entered,
              jumpToIndex: function (index) {
                capture(stream, index);
              },
              // handleJumpBottom
              jumpEnd: function () {
                capture(stream, stream.length);
              },
            });
          })
          .catch(function (e) {
            setFlash(extractError(e));
          });
      },
      footer: function (footer, closeModal) {
        submitButton = button("btn-primary", t("composer.change_reply_to.submit"));
        submitButton.disabled = true;
        submitButton.addEventListener("click", function () {
          if (!canSubmit()) {
            return;
          }
          opts.onSelect(selectedPost);
          closeModal();
        });
        footer.appendChild(submitButton);
        if (opts.currentPostNumber) {
          var remove = button("btn-danger", t("composer.change_reply_to.remove"));
          remove.addEventListener("click", function () {
            opts.onSelect(null);
            closeModal();
          });
          footer.appendChild(remove);
        }
        var cancel = button("", t("cancel"));
        cancel.addEventListener("click", function () {
          closeModal();
        });
        footer.appendChild(cancel);
      },
      onClose: function () {
        if (cleanup) {
          cleanup();
        }
      },
    });
    return close;
  }

  Discourse.changeReplyTo = { show: show };
})();
