var files = {};
var i;
for (i = 0; i < results.length; i++) {
  if (!(results[i].path in files)) {
    files[results[i].path] = [];
  }
  files[results[i].path].push(results[i]);
}

console.log(files);

$('body').append(nunjucks.render('static/templates/searchresults-template.html', {files: files}));

