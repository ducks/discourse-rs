// Follows one pointer (by its id) until it is released or canceled; returns
// the function that stops listening.
export default function trackPointer(startEvent, move, finish) {
  var events = ["pointermove", "pointerup", "pointercancel"];
  var handle = function (event) {
    if (event.pointerId === startEvent.pointerId) {
      (event.type === "pointermove" ? move : finish)(event);
    }
  };
  events.forEach(function (type) {
    document.addEventListener(type, handle);
  });
  return function () {
    events.forEach(function (type) {
      document.removeEventListener(type, handle);
    });
  };
}
