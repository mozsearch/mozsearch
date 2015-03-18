/*
function fetchData(ref)
{
  $.ajax(ref, {
    dataType: "json",
    success: function(data) {
      $('body').append(nunjucks.render('static/results-template.html', {files: data}));
    },
    error: function(jqXHR, error) {
      alert("XHR failed: " + error + ": " + ref);
    },
  });
}

var search = window.location.search;
if (search.startsWith("?")) {
  fetchData(search.substring(1));
}
*/

var data = [];
if (results.def && results.def.length) {
  data.push({kind: "Definitions", files: results.def});
}
if (results.assign && results.assign.length) {
  data.push({kind: "Assignments", files: results.assign});
}
if (results.use && results.use.length) {
  data.push({kind: "Uses", files: results.use});
}

$('body').append(nunjucks.render('static/results-template.html', {data: data}));

