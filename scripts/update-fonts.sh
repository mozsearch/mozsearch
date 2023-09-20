#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Usage:
#
# Run this script from the root of your searchfox directory, outside of your VM,
# so that we can open the fontello.com website in your browser to let you start
# from our existing configuration and you can pick new icons.
#
# We'll wait for you to hit enter, indicating you're done editing the config,
# then we'll download the new font you configured.  You should commit that and
# probably then everything is great.

# # Font Updating
#
# This script is an attempt to somewhat automate regeneration of our CSS fonts
# through "fontello.com".  This is just the continuation of decisions made
# during DXR to use this mechanism, and I am opting for minimal change.  One
# related decision, however, is that I am replacing the inline SVGs we use for
# the navigation panel to instead use the font.  I am presuming the coice for
# the inline SVGs was made because we didn't have an easy/documented way to
# add more icons to the CSS fonts.
#
# I'm completely okay with us changing how we do this in the future.
#
# ## Archaeology
#
# It seems that our CSS fonts were inherited from an early version of DXR where
# we used "fontello.com" to build CSS fonts.  As of now, when trying to update
# the fonts, we had fonts that included ["ok", "down-dir", "up-dir", "tree-2"]
# but where we only seem to use "down-dir" and we animate it rotating.  Through
# eyeballing it seems that these are from the "Font Awesome" font.
#
# ## The Fontello API / This script
#
# Documentation is at https://github.com/fontello/fontello#developers-api which
# explains the session mechanism.
#
# This script is derived from the Makefile cited in the docs above,
# https://gist.github.com/puzrin/5537065.  We're not using a Makefile because
# we've largely standardized on bash for our glue logic.

FONTELLO_ROOT=https://fontello.com

# POST our configuration to receive a session id that's good for 24 hours and
# will be written to `.fontello-sid`.  We only need to do this if we don't have
# a sufficiently recent SID.  We use `-mmin -1440` to express less than 24
# hours because `-mtime` quantizes to days, and we use less than and negation
# because we want to create the file if it doesn't exist in addition to it being
# too old.
if [ ! "$(find .fontello-sid -mmin -1440)" ]; then
    curl --silent --show-error --fail --output .fontello-sid \
        --form "config=@scripts/fontello-config.json" \
        ${FONTELLO_ROOT}
fi

FONTELLO_SID=$(cat .fontello-sid)

# Open the browser!
x-www-browser ${FONTELLO_ROOT}/${FONTELLO_SID}

read -p "Press enter when you're done updating the fontello config, or ctrl-c to bail."

rm -rf .fontello.src .fontello.zip

curl --silent --show-error --fail --output .fontello.zip \
	${FONTELLO_ROOT}/${FONTELLO_SID}/get

unzip .fontello.zip -d .fontello.src

FONT_ROOT=$(ls -d .fontello.src/fontello-*)
FONT_DIR=${FONT_ROOT}/font
CSS_DIR=${FONT_ROOT}/css
FONT_NAME=icons
SF_FONT=static/fonts
SF_CSS=static/css

# The config may have changed, update it.
mv ${FONT_ROOT}/config.json scripts/fontello-config.json

mv ${FONT_ROOT}/LICENSE.txt ${SF_FONT}
mv ${FONT_DIR}/${FONT_NAME}.eot ${SF_FONT}/icons.eot
mv ${FONT_DIR}/${FONT_NAME}.svg ${SF_FONT}/icons.svg
mv ${FONT_DIR}/${FONT_NAME}.ttf ${SF_FONT}/icons.ttf
mv ${FONT_DIR}/${FONT_NAME}.woff ${SF_FONT}/icons.woff
mv ${FONT_DIR}/${FONT_NAME}.woff2 ${SF_FONT}/icons.woff2
mv ${CSS_DIR}/icons.css ${SF_CSS}/font-icons.css

rm -rf .fontello.zip .fontello.src
# we intentionally leave the .fontello-sid around for now to avoid generating
# unnecessary server load.
