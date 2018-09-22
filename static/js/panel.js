$(function() {
    /**
     * Toggles the ARIA expanded and hidden attributes' state.
     *
     * @param Object elem The element on which to toggle the attribute.
     */
    function toggleAria(elem) {
        var expandedState = elem.attr('aria-expanded') === 'true' ? 'false' : 'true';
        var hiddenState = elem.attr('aria-hidden') === 'true' ? 'false' : 'true';
        elem.attr('aria-hidden', hiddenState);
        elem.attr('aria-expanded', expandedState);
    }

    $('#panel-toggle').click(function(event) {
        var panelContent = $('#panel-content');
        var icon = $('.navpanel-icon', this);

        icon.toggleClass('expanded');
        panelContent.toggle();
        toggleAria(panelContent);
    });

    var permalinkNode = $('.panel a[title="Permalink"]');
    permalinkNode = permalinkNode.length == 0 ? null : permalinkNode[0];

    function pushPermalink() {
	if (history.state && history.state.permalink) {
	    return;
	}
	if (permalinkNode) {
	    history.pushState(
		{ permalink: permalinkNode.href },
		window.title,
		permalinkNode.href
	    );
	}
    }

    if (permalinkNode) {
        permalinkNode.addEventListener('click', (event) => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
                return;
            }
            event.preventDefault();
            pushPermalink();
        });
    }

    document.documentElement.addEventListener('keypress', (event) => {
        if (event.altKey || event.ctrlKey || event.metaKey) {
            return;
        }
        var inputs = /input|select|textarea/i;
        if (inputs.test(event.target.nodeName)) {
            return;
        }
        switch (event.key) {
            case 'y':
            case 'Y':
                pushPermalink();
                break;
            case 'l':
            case 'L':
                var linkNode = $('.panel a[title="Log"]');
                if (linkNode.length) {
                    linkNode[0].click();
                }
                break;
            case 'r':
            case 'R':
                var linkNode = $('.panel a[title="Raw"]');
                if (linkNode.length) {
                    linkNode[0].click();
                }
                break;
        }
    }, {passive: true});
});
