/**
 * Output-specific helper functions that could just as easily live in output.js
 * since it's always evaluated after this file anyways.
 **/

function getSuffix(filename)
{
  let pos = filename.lastIndexOf(".");
  if (pos == -1) {
    return null;
  }
  return filename.slice(pos + 1).toLowerCase();
}

function chooseIcon(path)
{
  let suffix = getSuffix(path);
  return {
    "bmp": "bmp",
    "c": "c",
    "cpp": "cpp",
    "gif": "gif",
    "h": "h",
    "ico": "ico",
    "jpeg": "jpg",
    "jpg": "jpg",
    "js": "js",
    "jsm": "js",
    "png": "png",
    "py": "py",
    "svg": "svg",
  }[suffix] || "";
}

function isIconForImage(iconType)
{
  const IMAGE_TYPES = [
    "bmp",
    "gif",
    "ico",
    "jpg",
    "png",
    "svg",
  ];

  return IMAGE_TYPES.includes(iconType);
}
