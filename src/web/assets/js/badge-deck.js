(function() {
    var BADGE_PRIORITY = {
        'fm-badge-inj': 0,
        'fm-badge-unh': 1,
        'fm-badge-lst': 2,
        'fm-badge-loa': 3,
        'fm-badge-yth': 4
    };

    function initDecks() {
        document.querySelectorAll('.fm-badge-deck').forEach(function(deck) {
            if (deck.dataset.init) return;
            deck.dataset.init = '1';

            var badges = Array.from(deck.querySelectorAll('.fm-badge'));
            if (badges.length < 2) return;

            badges.sort(function(a, b) {
                return badgePriority(a) - badgePriority(b);
            });

            badges.forEach(function(b) { deck.appendChild(b); });
            deck.setAttribute('data-count', badges.length);
            setCollapsed(badges);

            deck.addEventListener('click', function(e) {
                e.stopPropagation();
                if (deck.classList.contains('open')) {
                    deck.classList.remove('open');
                    setCollapsed(badges);
                } else {
                    closeAll();
                    deck.classList.add('open');
                    setExpanded(badges);
                }
            });
        });
    }

    function badgePriority(el) {
        for (var i = 0; i < el.classList.length; i++) {
            var cls = el.classList[i];
            if (cls in BADGE_PRIORITY) return BADGE_PRIORITY[cls];
        }
        return 50;
    }

    function setCollapsed(badges) {
        var len = badges.length;
        badges.forEach(function(b, i) {
            b.style.transform = i === 0 ? '' : 'translateY(' + (i * 5) + 'px)';
            b.style.zIndex = String(len - i);
        });
    }

    function setExpanded(badges) {
        var offset = 0;
        badges.forEach(function(b, i) {
            if (i === 0) {
                b.style.transform = '';
                b.style.zIndex = '';
                offset = b.offsetWidth + 4;
            } else {
                b.style.transform = 'translateX(' + offset + 'px)';
                b.style.zIndex = '1';
                offset += b.offsetWidth + 4;
            }
        });
    }

    function closeAll() {
        document.querySelectorAll('.fm-badge-deck.open').forEach(function(deck) {
            deck.classList.remove('open');
            setCollapsed(Array.from(deck.querySelectorAll('.fm-badge')));
        });
    }

    document.addEventListener('click', closeAll);
    initDecks();
})();
