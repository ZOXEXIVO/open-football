/**
 * watchlist.js — the watchlist, kept in this browser. Entries are watch keys
 * (`{id}-{fingerprint}`) built by the server; only the id prefix is read here.
 */
(function () {
    'use strict';

    const STORAGE_KEY = 'of.watchlist';

    function read() {
        try {
            var raw = window.localStorage.getItem(STORAGE_KEY);
            var items = raw ? JSON.parse(raw) : [];
            if (!Array.isArray(items)) return [];
            return items.filter(function (k) { return typeof k === 'string'; });
        } catch (e) {
            return [];
        }
    }

    function write(keys) {
        try {
            window.localStorage.setItem(STORAGE_KEY, JSON.stringify(keys));
        } catch (e) {
            // Blocked or full storage leaves the list as it was.
        }
    }

    function idOf(key) {
        return key.split('-', 1)[0];
    }

    window.OfWatchlist = {
        keys: read,

        has: function (key) {
            return read().indexOf(key) !== -1;
        },

        // A different person may hold this id from an earlier game; the
        // stale key gives way to the player being added.
        add: function (key) {
            var keys = read();
            if (keys.indexOf(key) !== -1) return;
            var id = idOf(key);
            keys = keys.filter(function (k) { return idOf(k) !== id; });
            keys.push(key);
            write(keys);
        },

        remove: function (key) {
            write(read().filter(function (k) { return k !== key; }));
        },

        retain: function (kept) {
            write(read().filter(function (k) { return kept.indexOf(k) !== -1; }));
        }
    };
})();
