// Scrolls the nearest scrollable ancestor of `anchor` along `axis` ("X" or
// "Y") while a dragged pointer rests near its edge
// (static/prosemirror/lib/table/drag-autoscroll.js).
var EDGE_SIZE = 32;
var MAX_STEP = 18;

function edgeDelta(point, start, end) {
  if (point < start + EDGE_SIZE) {
    return -Math.ceil(MAX_STEP * Math.min((start + EDGE_SIZE - point) / EDGE_SIZE, 1));
  }
  if (point > end - EDGE_SIZE) {
    return Math.ceil(MAX_STEP * Math.min((point - (end - EDGE_SIZE)) / EDGE_SIZE, 1));
  }
  return 0;
}

function scrollTarget(anchor, axis, point) {
  var overflowProperty = axis === "X" ? "overflowX" : "overflowY";
  var scrollSize = axis === "X" ? "scrollWidth" : "scrollHeight";
  var clientSize = axis === "X" ? "clientWidth" : "clientHeight";

  for (var element = anchor; element; element = element.parentElement) {
    var overflow = getComputedStyle(element)[overflowProperty];
    if (!/(auto|scroll)/.test(overflow) || element[scrollSize] <= element[clientSize]) {
      continue;
    }

    var bounds = element.getBoundingClientRect();
    var start = axis === "X" ? bounds.left : bounds.top;
    var end = axis === "X" ? bounds.right : bounds.bottom;
    var delta = edgeDelta(point, start, end);
    // The browser clamps the write at either boundary, which covers both RTL
    // scrollLeft conventions; the frames stop once a write changes nothing.
    if (delta) {
      return { delta: delta, element: element };
    }
  }

  return null;
}

export default function dragAutoscroll(anchor, axis, onScroll) {
  var frame = null;
  var point = null;
  var stopped = false;

  var tick = function () {
    frame = null;
    if (stopped || point === null) {
      return;
    }

    var scroller = scrollTarget(anchor, axis, point);
    if (!scroller) {
      return;
    }

    var property = axis === "X" ? "scrollLeft" : "scrollTop";
    var before = scroller.element[property];
    scroller.element[property] += scroller.delta;
    var change = scroller.element[property] - before;

    if (change) {
      if (onScroll) {
        onScroll(change);
      }
      frame = requestAnimationFrame(tick);
    }
  };

  return {
    update: function (event) {
      point = axis === "X" ? event.clientX : event.clientY;
      if (frame === null) {
        frame = requestAnimationFrame(tick);
      }
    },

    stop: function () {
      stopped = true;
      if (frame !== null) {
        cancelAnimationFrame(frame);
        frame = null;
      }
    },
  };
}
