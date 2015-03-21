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

$('body').append(nunjucks.render('static/templates/results-template.html', {data: data}));

