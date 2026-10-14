// NavigationBar's mobile dropdown (showDropdown): the trigger in the list
// controls opens the nav items, from the template beside it, as a DMenu
// with modalForMobile, so a bottom sheet (float.js). A click in the list
// closes it, as DDropdownMenu's does.
(function () {
  "use strict";

  if (!Discourse.once("navigation-bar")) {
    return;
  }

  var open = null;

  document.addEventListener("click", function (event) {
    var trigger = event.target.closest(".list-control-toggle-link-trigger");
    if (!trigger) {
      return;
    }
    var template = trigger.parentElement.querySelector(
      "template.list-control-toggle-link-menu"
    );
    if (!template) {
      return;
    }
    if (open && open.expanded) {
      open.close();
      return;
    }
    var content = template.content.cloneNode(true).firstElementChild;
    var menu = Discourse.float.menu(trigger, {
      identifier: "list-control-toggle-link",
      modalForMobile: true,
      autofocus: true,
      content: content,
    });
    content.addEventListener("click", function () {
      menu.close();
    });
    open = menu;
  });
})();
