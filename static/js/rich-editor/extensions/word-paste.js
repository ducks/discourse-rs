// extensions/word-paste.js: HTML pasted from Microsoft Word (desktop or
// Word for the web) made standard before it is parsed: its list
// paragraphs as lists, its quote styles as blockquotes, its comments and
// tracked deletions dropped. to-markdown.js runs the same clean-up.

var MSO_LIST_CLASSES = ["MsoListParagraphCxSpFirst", "MsoListParagraphCxSpMiddle", "MsoListParagraphCxSpLast"];

// Word's quote-like paragraph styles ("Quote", "Intense Quote", "Block
// Text"): Word has no semantic blockquote. Bare indentation is too
// ambiguous to count. Desktop Word names the style in a class; Word for
// the web in data-ccp-parastyle, by its English name whatever the
// document's language.
var MSO_QUOTE_CLASSES = ["MsoQuote", "MsoIntenseQuote", "MsoBlockText"];
var WAC_QUOTE_STYLES = ["Quote", "Intense Quote", "Block Text"];
var WAC_QUOTE_SELECTOR = WAC_QUOTE_STYLES.map(function (style) {
  return '[data-ccp-parastyle="' + style + '"]';
}).join(", ");

// "mso-list:l0 level2 lfo1": the list level, from 1
function extractListLevel(element) {
  var style = element.getAttribute("style") || "";
  var match = style.match(/level(\d+)/);
  return match ? parseInt(match[1], 10) : 1;
}

function orderedMarker(markerText) {
  var numberedMatch = markerText.match(/^(\d+)[.)]/);
  if (numberedMatch) {
    return { isOrdered: true, startNumber: parseInt(numberedMatch[1], 10) };
  }
  // a., b., A., B.
  if (/^[a-zA-Z][.)]/.test(markerText)) {
    return { isOrdered: true, startNumber: 1 };
  }
  // roman numerals, larger ones too (L., C., D., M.)
  if (/^[ivxlcdmIVXLCDM]+[.)]/.test(markerText)) {
    return { isOrdered: true, startNumber: 1 };
  }
  return null;
}

// Ordered or not, from the list marker (before it is removed).
function detectListType(element) {
  var markerSpan = element.querySelector('[style*="mso-list:Ignore"]');
  var found;
  if (markerSpan) {
    found = orderedMarker(markerSpan.textContent.trim());
    if (found) {
      return found;
    }
  }

  // IE conditional comments hold the marker
  var conditionalMatch = element.innerHTML.match(/<!\[if !supportLists\]>([\s\S]*?)<!\[endif\]>/i);
  if (conditionalMatch) {
    found = orderedMarker(conditionalMatch[1].replace(/<[^>]*>/g, "").trim());
    if (found) {
      return found;
    }
  }

  return { isOrdered: false, startNumber: 1 };
}

// A list paragraph's content, less the marker Word writes into it.
function extractListItemContent(element) {
  var clone = element.cloneNode(true);

  clone.querySelectorAll('[style*="mso-list:Ignore"]').forEach(function (el) {
    el.remove();
  });

  var html = clone.innerHTML.replace(/<!\[if !supportLists\]>[\s\S]*?<!\[endif\]>/gi, "");

  var temp = document.createElement("div");
  temp.innerHTML = html;

  temp.querySelectorAll("span, o\\:p").forEach(function (el) {
    if (!el.textContent.trim() && !el.querySelector("img")) {
      el.remove();
    }
  });

  return temp.innerHTML.trim();
}

function newList(listTypeInfo) {
  var list = document.createElement(listTypeInfo.isOrdered ? "ol" : "ul");
  list.setAttribute("data-tight", "true");
  if (listTypeInfo.isOrdered && listTypeInfo.startNumber !== 1) {
    list.setAttribute("start", String(listTypeInfo.startNumber));
  }
  return list;
}

// Word's list paragraphs as lists, in place.
export function transformWordLists(container) {
  var paragraphs = Array.prototype.slice.call(
    container.querySelectorAll(
      MSO_LIST_CLASSES.map(function (c) {
        return "p." + c;
      }).join(", ")
    )
  );

  if (paragraphs.length === 0) {
    return;
  }

  var currentList = null;
  // {list, level, isOrdered} for the nesting
  var listStack = [];
  var lastLevel = 0;

  paragraphs.forEach(function (p) {
    var level = extractListLevel(p);
    // the type before the content, which drops the marker
    var listTypeInfo = detectListType(p);
    var content = extractListItemContent(p);
    var isFirst = p.className.indexOf("MsoListParagraphCxSpFirst") >= 0;
    var isLast = p.className.indexOf("MsoListParagraphCxSpLast") >= 0;

    var li = document.createElement("li");
    li.innerHTML = content;

    if (isFirst || !currentList) {
      currentList = newList(listTypeInfo);
      listStack = [{ list: currentList, level: 1, isOrdered: listTypeInfo.isOrdered }];
      lastLevel = 1;

      p.parentNode.insertBefore(currentList, p);
    }

    if (level > lastLevel) {
      // deeper, as this item's type
      for (var i = lastLevel; i < level; i++) {
        var nestedList = newList(listTypeInfo);
        var parentList = listStack[listStack.length - 1].list;
        var lastItem = parentList.lastElementChild;

        if (lastItem) {
          lastItem.appendChild(nestedList);
        } else {
          parentList.appendChild(nestedList);
        }

        listStack.push({ list: nestedList, level: i + 1, isOrdered: listTypeInfo.isOrdered });
      }
    } else if (level < lastLevel) {
      while (listStack.length > 1 && listStack[listStack.length - 1].level > level) {
        listStack.pop();
      }
    }

    listStack[listStack.length - 1].list.appendChild(li);

    lastLevel = level;

    p.remove();

    if (isLast) {
      currentList = null;
      listStack = [];
      lastLevel = 0;
    }
  });
}

function isQuoteParagraph(node) {
  if (!node || node.nodeType !== Node.ELEMENT_NODE || node.tagName !== "P") {
    return false;
  }

  return (
    MSO_QUOTE_CLASSES.some(function (cls) {
      return node.classList.contains(cls);
    }) || node.querySelector(WAC_QUOTE_SELECTOR) !== null
  );
}

function nextElementSibling(node) {
  var sibling = node.nextSibling;
  while (sibling && sibling.nodeType !== Node.ELEMENT_NODE) {
    sibling = sibling.nextSibling;
  }
  return sibling;
}

// Word's quote paragraphs as blockquotes, consecutive ones in one, in
// place.
export function transformWordQuotes(container) {
  var candidates = new Set();

  container
    .querySelectorAll(
      MSO_QUOTE_CLASSES.map(function (c) {
        return "p." + c;
      }).join(", ")
    )
    .forEach(function (p) {
      candidates.add(p);
    });

  container.querySelectorAll(WAC_QUOTE_SELECTOR).forEach(function (run) {
    var p = run.closest("p");
    if (p) {
      candidates.add(p);
    }
  });

  if (candidates.size === 0) {
    return;
  }

  // in document order, so merging consecutive siblings is stable
  var paragraphs = Array.prototype.slice.call(container.querySelectorAll("p")).filter(function (p) {
    return candidates.has(p);
  });

  var absorbed = new Set();

  paragraphs.forEach(function (paragraph) {
    if (absorbed.has(paragraph)) {
      return;
    }

    var blockquote = document.createElement("blockquote");
    paragraph.parentNode.insertBefore(blockquote, paragraph);

    var current = paragraph;
    while (isQuoteParagraph(current)) {
      var next = nextElementSibling(current);
      absorbed.add(current);
      MSO_QUOTE_CLASSES.forEach(function (cls) {
        current.classList.remove(cls);
      });
      if (current.classList.length === 0) {
        current.removeAttribute("class");
      }
      blockquote.appendChild(current);
      current = next;
    }
  });
}

// Review markup that never belongs in a post: comment anchors and
// markers, the comment list Word appends, and tracked deletions. Manual
// strikethrough comes as <s>, not <del>, so <del> is only a revision.
var WORD_REVIEW_SELECTORS = [
  "a.msocomanchor",
  "span.MsoCommentReference",
  "[style*='mso-comment-reference']",
  "[style*='mso-special-character:comment']",
  "a[href^='#_msocom']",
  "a[name^='_msocom']",
  "hr.msocomoff",
  "[style*='mso-element:comment-list']",
  "[style*='mso-element:comment']",
  "p.MsoCommentText",
  "p.MsoCommentSubject",
  "del",
].join(", ");

// Word for the web has no mso-* markers but these classes and data-ccp-*
// attributes, anchored to their form so other HTML naming the words is not
// taken for Word.
var WAC_MARKERS = /class=["'][^"']*\b(?:OutlineElement|NormalTextRun|WACImageContainer)\b|data-ccp-[\w-]*=/;

function isWordHtml(html) {
  return (
    html.indexOf("mso-") >= 0 ||
    html.indexOf("urn:schemas-microsoft-com") >= 0 ||
    /class=["']?Mso/.test(html) ||
    WAC_MARKERS.test(html)
  );
}

export function stripWordReviewMarkup(container) {
  container.querySelectorAll(WORD_REVIEW_SELECTORS).forEach(function (el) {
    el.remove();
  });
}

// Word's HTML made standard: lists, quote styles, and no review markup.
export function transformWordHtml(html) {
  var hasLists = MSO_LIST_CLASSES.some(function (cls) {
    return html.indexOf(cls) >= 0;
  });
  var hasQuotes =
    MSO_QUOTE_CLASSES.some(function (cls) {
      return html.indexOf(cls) >= 0;
    }) ||
    WAC_QUOTE_STYLES.some(function (style) {
      return html.indexOf('data-ccp-parastyle="' + style + '"') >= 0;
    });
  var isWord = isWordHtml(html);

  if (!hasLists && !hasQuotes && !isWord) {
    return html;
  }

  var doc = new DOMParser().parseFromString(html, "text/html");

  if (hasLists) {
    transformWordLists(doc.body);
  }

  if (hasQuotes) {
    transformWordQuotes(doc.body);
  }

  if (isWord) {
    stripWordReviewMarkup(doc.body);
  }

  return doc.body.innerHTML;
}

export default {
  plugins: function (params) {
    return new params.pmState.Plugin({
      props: {
        transformPastedHTML: function (html) {
          return transformWordHtml(html);
        },
      },
    });
  },
};
