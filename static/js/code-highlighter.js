/* jshint devel:true, esnext: true */
/* globals nunjucks: true, $ */

/**
 * This file consists of four major pieces of functionality for a file view:
 * 0) Any combination of 1) and 2)
 * 1) Multi-select highlight lines with shift key and update window.location.hash
 * 2) Multi-select highlight lines with command/control key and update window.location.hash
 * 3) Highlight lines when page loads, if window.location.hash exists
 */

let previouslyStuckElements = [];

/**
 * Hacky but workable sticky detection logic.
 *
 * document.elementsFromPoint can give us the stack of all of the boxes that
 * occur at a given point in the viewport.  The naive assumption that we can
 * look at the stack of returned elements and see if there are two
 * "source-line-with-number" in the stack (one for the sticky bit, one for the
 * actual source it's occluding) turns out to run into problems when sticky
 * things start or stop stickying.  Also, you potentially have to probe twice
 * with a second offset to compensate for exlusive box boundary issues.
 *
 * So instead we can leverage some important facts:
 * - Our sticky lines line up perfectly.  They're always fully visible.
 *   (That said, given that fractional pixel sizes can happen with scaling and
 *   all that, it's likely smart to avoid doing exact math on that.)
 * - We've annotated every line with line numbers.  So any discontinuity greater
 *   than 1 is an indication of a stuck line.  Unfortunately, since we also
 *   expect that sometimes there will be consecutive stuck lines, we can't treat
 *   lack of discontinuity as proof that things aren't stuck.  However, we can
 *   leverage math by making sure that a line beyond our maximum nesting level's
 *   line number lines up.
 */
$("#scrolling").on('scroll', function() {
  // Our logic can't work on our diff output because there will be line number
  // discontinuities and line numbers that are simply missing.
  const contentElem = document.getElementById('content');
  if (contentElem.classList.contains('diff')) {
    return;
  }

  const scrolling = document.getElementById('scrolling');
  const firstSourceY = scrolling.offsetTop;
  // The goal is to make sure we're in the area the source line numbers live.
  const lineForSizing = document.querySelector('.line-number');
  const sourceLinesX = lineForSizing.offsetLeft + 6;
  const lineHeight = lineForSizing.offsetHeight;

  const MAX_NESTING = 10;

  let firstStuck = null;
  let lastStuck = null;
  const jitter = 6;

  function extractLineNumberFromElem(elem) {
    if (!elem.classList.contains('line-number')) {
      return null;
    }

    let num = parseInt(elem.id.slice(1), 10);
    if (isNaN(num) || num < 0) {
      return null;
    }

    return num;
  }

  /**
   * Walk at a fixed offset into the middle of what should be stuck line
   * numbers.
   *
   * If we don't find a line-number, then we expect that to be due to the
   * transition from stuck elements to partially-scrolled source lines.  It
   * means the current set of lines are all stuck.
   *
   * If we do find a line-number, then we have to look at the actual line
   * number.  If it's consecutive with the previous line, then it means the
   * previous line AND this line are both not stuck, and we should return
   * what we had exclusive of the previous line.
   */
  function walkLinesAndGetLines() {
    let offset = 6;
    let sourceLines = [];

    // Find a line number that can't possibly be stuck.
    let sanityCheckLineNum =
      extractLineNumberFromElem(document.elementFromPoint(
        sourceLinesX, firstSourceY + offset + lineHeight * MAX_NESTING));
    // if we didn't find a line, try again with a slight jitter because there
    // might have been a box boundary edge-case.
    if (!sanityCheckLineNum) {
      sanityCheckLineNum =
        extractLineNumberFromElem(document.elementFromPoint(
          sourceLinesX, jitter + firstSourceY + offset + lineHeight * MAX_NESTING));
    }

    // If we couldn't find a sanity-checking line number, then either our logic
    // is broken or the file doesn't have enough lines to necessitate the sticky
    // logic.  Just bail.
    if (!sanityCheckLineNum) {
      return sourceLines;
    }

    for (let iLine=0; iLine <= MAX_NESTING; iLine++) {
      let elem = document.elementFromPoint(sourceLinesX, firstSourceY + offset);
      if (!elem || !elem.classList.contains('line-number')) {
        break;
      }

      let lineNum = parseInt(elem.id.slice(1), 10);

      let expectedLineNum = sanityCheckLineNum - MAX_NESTING + iLine;
      if (lineNum !== expectedLineNum) {
        // the line-number's parent is the source-line-with-number
        sourceLines.push(elem.parentElement);
      } else {
        break;
      }

      offset += lineHeight;
    }

    return sourceLines;
  }

  let newlyStuckElements = walkLinesAndGetLines();

  let noLongerStuck = new Set(previouslyStuckElements);
  for (let elem of newlyStuckElements) {
    elem.classList.add('stuck');
    noLongerStuck.delete(elem);
  }
  let lastElem = null;
  if (newlyStuckElements.length) {
    lastElem = newlyStuckElements[newlyStuckElements.length - 1];
  }
  let prevLastElem = null;
  if (previouslyStuckElements.length) {
    prevLastElem = previouslyStuckElements[previouslyStuckElements.length - 1];
  }
  if (lastElem !== prevLastElem) {
    if (prevLastElem) {
      prevLastElem.classList.remove('last-stuck');
    }
    if (lastElem) {
      lastElem.classList.add('last-stuck');
    }
  }

  for (let elem of noLongerStuck) {
    elem.classList.remove('stuck');
  }

  previouslyStuckElements = newlyStuckElements;
});

$(function () {
  'use strict';
  var container = $('#file');
  var lastModifierKey = null; // use this as a sort of canary/state indicator showing the last user action
  var singleLinesArray = []; //track single highlighted lines here
  var rangesArray = []; // track ranges of highlighted lines here

  //sort a one dimensional array in Ascending order
  function sortAsc(a, b) {
    return a - b;
  }
  function stringToRange(a) {
    a = a.split('-');
    a[0] = parseInt(a[0],10);
    a[1] = parseInt(a[1],10);
    return a;
  }
  function sortRangeAsc(a, b) {
    // tweak in order to account for inverted ranges like 150-120
    return Math.min(a[0],a[1]) - Math.min(b[0],b[1]);
  }
  function lineFromId(id) {
    if (id) {
      return id.slice(1);
    }
    return id;
  }
  /**
   * Scours the current DOM and returns [array of all single-selected line
   * Numbers, array of all multi-selected line Numbers] suitable for
   * de-duplication and range-derivation.
   */
  function generateSelectedArrays() {
    var line = null;
    var rangeMax = null;
    var lines = [];
    var rangesArray = [];
    var singleLinesArray = [];

    var multiSelected = $('.line-number.multihighlight');
    var singleSelected = $('.line-number.highlighted');

    function generateLines(selected, lines) {
      for (var i = 0; i < selected.length; i++) {
        if (selected[i].id) {
          lines.push(parseInt(lineFromId(selected[i].id), 10));
        }
      }
      return lines;
    }

    lines = generateLines(multiSelected, lines);
    lines = generateLines(singleSelected, lines);

    // strip all single lines, e.g. those without an adjacent line+1 == nextLine
    for (var s = lines.length - 1; s >= 0; s--) {
      line = lines[s];
      // this presumes selected is sorted in asc order, if not it won't work
      if (line !== lines[s + 1] - 1 && line !== lines[s - 1] + 1) {
        singleLinesArray.push(line);
        lines.splice(s, 1);
      }
    }

    //this presumes selected is sorted in asc order after single lines have been removed
    while (lines.length > 0) {
      line = lines[0];
      var pos = 1;
      while (line === lines[pos] - pos) {
        rangeMax = lines[pos];
        pos++;
      }
      rangesArray.push([line, rangeMax]);
      lines.splice(0, pos);
    }
    //return sorted arrays
    return [singleLinesArray.sort(sortAsc), rangesArray.sort(sortRangeAsc)];
  }

  /**
   * Reflects the current state of selected lines per the DOM into the location
   * hash, updating the current history entry.
   */
  function setWindowHash() {
    var windowHash = null;
    var s = null;
    var r = null;
    var reCleanup = /(^#?,|,$)/;
    var selectedArray = generateSelectedArrays(); // generates sorted arrays
    var singleLinesArray = selectedArray[0];
    var rangesArray = selectedArray[1];
    // eliminate duplication
    for (s = 0; s < singleLinesArray.length; s++) {
      for (r = 0; r < rangesArray.length; r++) {
        if (s >= rangesArray[r][0] && s <= rangesArray[r][1]) {
          singleLinesArray.splice(s,1);
          s--;
        }
      }
    }
    if (singleLinesArray.length || rangesArray.length) {
      windowHash = '#';
    }
    for (s = 0, r = 0; s < singleLinesArray.length || r < rangesArray.length;) {
      // if no ranges left or singleLine < range add singleLine to hash
      // if no singleLines left or range < singleLine add range to hash
      if ((r == rangesArray.length) || (singleLinesArray[s] < rangesArray[r][0])) {
        windowHash += singleLinesArray[s] + ',';
        s++;
      } else if (( s == singleLinesArray.length) || (rangesArray[r][0] < singleLinesArray[s])) {
        windowHash += rangesArray[r][0] + '-' + rangesArray[r][1] + ',';
        r++;
      }
    }
    if (windowHash) {
      windowHash = windowHash.replace(reCleanup, '');
      history.replaceState(null, '', windowHash);
      scrollIntoView(windowHash.slice(1), false);

      $("a[data-update-link=true]").each(function(i, elt) {
        $(elt).attr("href", $(elt).attr("data-link") + windowHash);
      });
    }
  }

  //parse window.location.hash on new requsts into two arrays
  //one of single lines and one multilines
  //use with singleLinesArray and rangesArray for adding/changing new highlights
  function getSortedHashLines() {
    var highlights = window.location.hash.substring(1);
    var lineStart = null;
    var reRanges = /[0-9]+-[0-9]+/g;
    var reCleanup = /[^0-9,]/g;
    var ranges = null;
    var firstRange = null;
    highlights = highlights.replace(/ /g,''); // clean whitespace
    ranges = highlights.match(reRanges);
    if (ranges !== null) {
      ranges = ranges.map(stringToRange).sort(sortRangeAsc);
      //strip out multiline items like 12-15, so that all that is left are single lines
      //populate rangesArray for reuse if a user selects more ranges later
      for (var i = 0; i < ranges.length; i++) {
        highlights = highlights.replace(ranges[i].join('-'), '');
        ranges[i].sort(sortAsc);
      }
      // add the ordered ranges to the rangesArray
      rangesArray = rangesArray.concat(ranges);
      firstRange = rangesArray[0];
      highlights = highlights.replace(reCleanup ,''); // clean anything other than digits and commas
      highlights = highlights.replace(/,,+/g, ','); // clean multiple commas
      highlights = highlights.replace(/^,|,$/g, ''); // clean leading and tailing comas
    }

    if (highlights.length) {
      //make an array of integers and sort it for the remaining single lines
      highlights = highlights.split(',');
      for (var h = 0; h < highlights.length; h++) {
        highlights[h] = parseInt(highlights[h], 10);
        //in case some unwanted string snuck by remove it
        if (isNaN(highlights[h])) {
          highlights.splice(h,1);
          h--;
        }
      }
      highlights = highlights.sort(sortAsc);
      //set the global singleLinesArry for reuse
      singleLinesArray = highlights;
    } else {
      //this happens if there is no single line in a url
      //without setting this the url gets an NaN element in it
      highlights = null;
    }

    //a url can look like foo#12,15,20-25 or foo#12-15,18,20-25 or foo#1,2,3 etc.
    //the lineStart should be the smallest integer in the single or highlighted ranges
    //this ensures a proper position to which to scroll once the page loads
    if (firstRange !== null && highlights !== null) {
      if (highlights[0] < firstRange[0]) {
        lineStart = highlights[0];
      } else if (highlights[0] > firstRange[0]) {
        lineStart = firstRange[0];
      }
    } else if (firstRange !== null && highlights === null) {
      lineStart = firstRange[0];
    } else if (firstRange === null && highlights !== null) {
      lineStart = highlights[0];
    } else {
      lineStart = null;
    }

    return {'lineStart':lineStart, 'highlights':singleLinesArray, 'ranges':rangesArray};
  }

  //first bind to all .line-number click events only
  container.on('click', '.line-number', function (event) {
    // hacky logic to jump if this is a stuck line number
    if (!event.shiftKey && !event.ctrlKey) {
      var containingLine = $(this).closest('.source-line-with-number')[0];
      if (containingLine) {
        if (containingLine.classList.contains('stuck')) {
          document.getElementById('scrolling').scrollTop -= containingLine.offsetTop;
          return;
        }
      }
    }

    var clickedNum = parseInt(lineFromId($(this).attr('id')), 10); // get the clicked line number
    var line = $('#l' + clickedNum + ', #line-' + clickedNum); // get line-number and code
    var lastSelectedLine = $('.line-number.last-selected, .source-line.last-selected');
    var lastSelectedNum = parseInt(lineFromId($('.line-number.last-selected').attr('id')), 10); // get last selected line number as integer
    var selectedLineNums = null; // used for selecting elements with class .line-number
    var selectedLineCode = null; // used for selecting code elements in .code class
    var self = this;

    function pickLineNums(lowInclusive, highExclusive) {
      let count = highExclusive - lowInclusive;
      let ids = new Array(count);
      for (let i = 0; i < count; i++) {
        ids[i] = `#l${lowInclusive + i}`;
      }
      return $(ids.join(', '));
    }
    function pickLineCodes(lowInclusive, highExclusive) {
      let count = highExclusive - lowInclusive;
      let ids = new Array(count);
      for (let i = 0; i < count; i++) {
        ids[i] = `#line-${lowInclusive + i}`;
      }
      return $(ids.join(', '));
    }

    //multiselect on shiftkey modifier combined with click
    if (event.shiftKey) {
      var classToAdd = 'multihighlight';
      // on shift, find last-selected code element
      // if lastSelectedNum less than clickedNum go back
      // else if lastSelectedNum greater than line id, go forward
      if (lastSelectedNum === clickedNum) {
        //toggle a single shiftclicked line
        line.removeClass('last-selected highlighted clicked multihighlight');
      } else if (lastSelectedNum < clickedNum) {
        //shiftclick descending down the page
        line.addClass('clicked');
        selectedLineNums = pickLineNums(lastSelectedNum, clickedNum);
        selectedLineCode = pickLineCodes(lastSelectedNum, clickedNum);
        $('.last-selected').removeClass('clicked');
      } else if (lastSelectedNum > clickedNum) {
        //shiftclick ascending up the page
        $('.line-number, .source-line').removeClass('clicked');
        line.addClass('clicked');
        selectedLineNums = pickLineNums(clickedNum, lastSelectedNum);
        selectedLineCode = pickLineCodes(clickedNum, lastSelectedNum);
      }
      selectedLineNums.addClass(classToAdd);
      selectedLineCode.addClass(classToAdd);

      //set the last used modifier key
      lastModifierKey = 'shift';
      // since all highlighed items are stripped, add one back, mark new last-selected
      lastSelectedLine.addClass(classToAdd)
        .removeClass('last-selected highlighted');
      //line.removeClass('highlighted');
      line.addClass(classToAdd);
      line.addClass('last-selected');

    } else if (event.shiftKey && lastModifierKey === 'singleSelectKey') {
      //if ctrl/command was last pressed, add multihighlight class to new lines
      $('.line-number, .source-line').removeClass('clicked');
      line.addClass('clicked');
      selectedLineNums = pickLineNums(lastSelectedNum, clickedNum);
      selectedLineNums.addClass('multihighlight')
        .removeClass('highlighted');
      selectedLineCode = pickLineCodes(lastSelectedNum, clickedNum);
      selectedLineCode.addClass('multihighlight')
        .removeClass('highlighted');
      line.addClass('multihighlight');

    } else if (event.ctrlKey || event.metaKey) {
      //a single click with ctrl/command highlights one line and preserves existing highlights
      lastModifierKey = 'singleSelectKey';
      $('.highlighted').addClass('multihighlight');
      $('.line-number, .source-line').removeClass('last-selected clicked highlighted');
      if (lastSelectedNum !== clickedNum) {
        line.toggleClass('clicked last-selected multihighlight');
      } else {
        line.toggleClass('multihighlight');
        history.replaceState(null, '', '#');
      }

    } else {
      //set lastModifierKey ranges and single lines to null, then clear all highlights
      lastModifierKey = null;
      //Remove existing highlights.
      $('.line-number, .source-line').removeClass('last-selected highlighted multihighlight clicked');
      //empty out single lines and ranges arrays
      rangesArray = [];
      singleLinesArray = [];
      //toggle highlighting on for any line that was not previously clicked
      if (lastSelectedNum !== clickedNum) {
        //With this we're one better than github, which doesn't allow toggling single lines
        line.toggleClass('last-selected highlighted');
      } else {
        history.replaceState(null, '', '#');
      }
    }
    setWindowHash();
  });

  //highlight line(s) if someone visits a url directly with an #anchor
  $(document).ready(function () {
    if (window.location.hash.substring(1)) {
      var toHighlight = getSortedHashLines(),
      jumpPosition = $('#l' + toHighlight.lineStart).offset(),
      highlights = toHighlight.highlights,
      ranges = toHighlight.ranges;

      if (highlights !== null) {
        //add single line highlights
        for (var i=0; i < highlights.length; i++) {
          $('#l' + highlights[i] + ', #line-' + highlights[i]).addClass('highlighted');
        }
      }

      if (ranges !== null) {
        //handle multiple sets of multi-line highlights from an incoming url
        for (var j=0; j < ranges.length; j++) {
          //handle a single set of line ranges here; the c counter must be <= since it is a line id
          for (var c = ranges[j][0]; c <= ranges[j][1]; c++) {
            $('#l' + c + ', #line-' + c).addClass('highlighted');
          }
        }
      }

      //for directly linked line(s), scroll to the offset minus 150px for fixed search bar height
      //but only scrollTo if the offset is more than 150px in distance from the top of the page
      jumpPosition = parseInt(jumpPosition.top, 10) - 150;
      if (jumpPosition >= 0) {
        document.getElementById('scrolling').scrollTo(0, jumpPosition);
      } else {
        document.getElementById('scrolling').scrollTo(0, 0);
      }
      //tidy up an incoming url that might be typed in manually
      setWindowHash();
    }
  });

});
