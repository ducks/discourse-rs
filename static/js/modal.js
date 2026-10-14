// DModal (ui-kit/d-modal.gts) as the modal service shows it, for the
// decorators' fullscreen views:
//
//   Discourse.modal.show({ className, title, body: fn(bodyEl, close),
//                          footer: fn(footerEl, close), onClose,
//                          hideHeader, ariaLabel, attributes })
//       renders the modal into .modal-container (ModalContainer, the
//       application template's, inside .discourse-root) with html.modal-open,
//       and returns its close function. body fills .d-modal__body, footer
//       a .d-modal__footer after it (the <:footer> block); onClose runs
//       once it is gone. A modal shown while one is open replaces it.
//
// Dismissable as DModal is when given @closeModal: the close button,
// Escape and a click on the backdrop close it, after the exit animation.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.modal) {
    return;
  }

  var FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  var active = null;

  function container() {
    var el = document.querySelector(".modal-container");
    if (!el) {
      el = document.createElement("div");
      el.className = "modal-container";
      (document.querySelector(".discourse-root") || document.body).appendChild(el);
    }
    return el;
  }

  // waitForAnimationEnd: resolves at once when nothing animates.
  function animationEnd(el) {
    return new Promise(function (resolve) {
      var animations = el.getAnimations ? el.getAnimations() : [];
      if (!animations.length) {
        resolve();
        return;
      }
      Promise.all(
        animations.map(function (a) {
          return a.finished.catch(function () {});
        })
      ).then(function () {
        resolve();
      });
    });
  }

  function show(opts) {
    if (active) {
      active.remove();
    }
    var t = Discourse.cooked ? Discourse.cooked.t : function (k) { return k; };
    var icon = Discourse.cooked ? Discourse.cooked.icon : function () { return ""; };

    var modal = document.createElement("div");
    if (opts.ariaLabel) {
      modal.setAttribute("aria-label", opts.ariaLabel);
    } else if (!opts.hideHeader) {
      modal.setAttribute("aria-labelledby", "discourse-modal-title");
    }
    modal.setAttribute("aria-modal", "true");
    modal.className = "modal d-modal" + (opts.className ? " " + opts.className : "");
    modal.setAttribute("data-keyboard", "false");
    modal.setAttribute("role", "dialog");
    Object.keys(opts.attributes || {}).forEach(function (name) {
      modal.setAttribute(name, opts.attributes[name]);
    });
    // @hideHeader leaves out the title and the close button.
    modal.innerHTML =
      '<div class="d-modal__container">' +
      (opts.hideHeader
        ? ""
        : '<div class="d-modal__header">' +
          '<div class="d-modal__title"><h1 class="d-modal__title-text" id="discourse-modal-title"></h1></div>' +
          '<button class="btn no-text btn-icon btn-transparent modal-close" type="button">' +
          icon("xmark") +
          '<span aria-hidden="true">&#8203;</span></button></div>') +
      '<div class="d-modal__body" tabindex="-1"></div></div>';
    var closeButton = modal.querySelector(".modal-close");
    if (closeButton) {
      modal.querySelector(".d-modal__title-text").textContent = opts.title || "";
      closeButton.title = t("modal.close");
    }
    var modalContainer = modal.querySelector(".d-modal__container");
    var body = modal.querySelector(".d-modal__body");

    var backdrop = document.createElement("div");
    backdrop.className = "d-modal__backdrop";

    var closing = false;
    var removed = false;

    function remove() {
      if (removed) {
        return;
      }
      removed = true;
      document.documentElement.removeEventListener("keydown", onKeydown, { capture: true });
      modal.remove();
      backdrop.remove();
      if (!document.querySelector(".d-modal")) {
        document.documentElement.classList.remove("modal-open");
      }
      if (active && active.modal === modal) {
        active = null;
      }
      if (opts.onClose) {
        opts.onClose();
      }
    }

    // #animatePopOff, then @closeModal.
    function close() {
      if (closing || removed) {
        return;
      }
      closing = true;
      modal.classList.add("is-animating");
      modalContainer.classList.add("is-exiting");
      backdrop.classList.add("is-exiting");
      Promise.all([animationEnd(modalContainer), animationEnd(backdrop)]).then(remove);
    }

    function onKeydown(event) {
      // Keys go no further than the modal (handleDocumentKeydown).
      if (!modal.contains(document.activeElement)) {
        event.stopPropagation();
        event.preventDefault();
      }
      if (event.key === "Escape") {
        event.stopPropagation();
        close();
        return;
      }
      // dTrapTab: Tab cycles through the modal's focusable elements.
      if (event.key === "Tab") {
        var focusable = Array.prototype.filter.call(modal.querySelectorAll(FOCUSABLE), function (el) {
          return el.offsetParent !== null;
        });
        if (!focusable.length) {
          event.preventDefault();
          return;
        }
        var first = focusable[0];
        var last = focusable[focusable.length - 1];
        if (event.shiftKey && (document.activeElement === first || !modal.contains(document.activeElement))) {
          last.focus();
          event.preventDefault();
        } else if (!event.shiftKey && (document.activeElement === last || !modal.contains(document.activeElement))) {
          first.focus();
          event.preventDefault();
        }
      }
    }

    if (closeButton) {
      closeButton.addEventListener("click", close);
    }
    backdrop.addEventListener("click", function (event) {
      if (event.button === 0) {
        close();
      }
    });
    // Keeps the hamburger menu open under a backdrop click.
    backdrop.addEventListener("pointerdown", function (event) {
      event.stopPropagation();
    });

    if (opts.body) {
      opts.body(body, close);
    }
    if (opts.footer) {
      var footer = document.createElement("div");
      footer.className = "d-modal__footer";
      modalContainer.appendChild(footer);
      opts.footer(footer, close);
    }

    var target = container();
    target.appendChild(modal);
    target.appendChild(backdrop);
    document.documentElement.classList.add("modal-open");
    document.documentElement.addEventListener("keydown", onKeydown, { capture: true });

    // dTrapTab's autofocus: an [autofocus] element, else the body.
    var autofocus = modal.querySelector("[autofocus]");
    (autofocus || body).focus({ preventScroll: true });

    // setupModal: the entering animation.
    modal.classList.add("is-animating");
    modalContainer.classList.add("is-entering");
    animationEnd(modalContainer).then(function () {
      modalContainer.classList.remove("is-entering");
      modal.classList.remove("is-animating");
    });

    active = { modal: modal, remove: remove };
    return close;
  }

  Discourse.modal = { show: show };
})();
