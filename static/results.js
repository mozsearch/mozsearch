function fetchData(ref)
{
  $.ajax(ref, {
    dataType: "json",
    success: function(data) {
      $('body').append(nunjucks.render('resources/results-template.html', {files: data}));
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
