/**
 * fm-dialog.js — Reusable dialog component for Open Football
 *
 * Usage:
 *   OpenFootballDialog.open({
 *     title: 'Transfer Player',
 *     fields: [
 *       { name: 'club_id', label: 'Club', type: 'select', options: [{value:'1', text:'Arsenal'}] },
 *       { name: 'fee', label: 'Fee ($)', type: 'number', placeholder: '0' },
 *     ],
 *     confirmText: 'Transfer',
 *     cancelText: 'Cancel',
 *     onConfirm: (data) => { console.log(data); },
 *   });
 *
 *   OpenFootballDialog.confirm({
 *     title: 'Clear Injury',
 *     message: 'Remove this player\'s injury?',
 *     confirmText: 'Clear',
 *     onConfirm: () => { ... },
 *   });
 *
 * An autocomplete field can remember what was picked last time. Give it a
 * recents store and the field grows a row of recent values between its label
 * and its input — one click fills the field, no menu in between:
 *
 *   { name: 'to_club_id', type: 'autocomplete', url: '/api/clubs',
 *     recents: { key: 'destination-club', label: 'Recent', exclude: '42' } }
 *
 * Fields sharing a `key` share one list, so the transfer and loan dialogs
 * offer the same destination clubs. `exclude` drops one value from the row
 * (the club the player is already at). Entries are written on confirm.
 */
(function () {
    'use strict';

    let backdrop = null;
    let dialog = null;

    const RECENTS_PREFIX = 'of.recents.';
    const RECENTS_LIMIT = 4;

    function ensureDOM() {
        if (backdrop) return;

        backdrop = document.createElement('div');
        backdrop.className = 'fm-dlg-backdrop';
        backdrop.addEventListener('click', function (e) {
            if (e.target === backdrop) close();
        });

        dialog = document.createElement('div');
        dialog.className = 'fm-dlg';
        backdrop.appendChild(dialog);
        document.body.appendChild(backdrop);
    }

    /** Recently picked values for one store key, most recent first. */
    function readRecents(key) {
        try {
            var raw = window.localStorage.getItem(RECENTS_PREFIX + key);
            var items = raw ? JSON.parse(raw) : [];
            if (!Array.isArray(items)) return [];
            return items.filter(function (i) {
                return i && i.value !== undefined && i.value !== '' && i.label;
            });
        } catch (e) {
            return [];
        }
    }

    function writeRecent(key, entry) {
        try {
            var kept = readRecents(key).filter(function (i) {
                return String(i.value) !== String(entry.value);
            });
            kept.unshift(entry);
            window.localStorage.setItem(
                RECENTS_PREFIX + key,
                JSON.stringify(kept.slice(0, RECENTS_LIMIT))
            );
        } catch (e) {
            // Storage can be full or blocked. Recents are a shortcut, not state.
        }
    }

    function fieldRecents(f) {
        if (!f.recents || !f.recents.key) return [];
        var exclude = f.recents.exclude === undefined || f.recents.exclude === null
            ? null
            : String(f.recents.exclude);
        return readRecents(f.recents.key).filter(function (i) {
            return String(i.value) !== exclude;
        });
    }

    /** Drop the applied state once the field no longer holds that value. */
    function clearRecentSelection(row) {
        if (!row) return;
        Array.prototype.forEach.call(row.querySelectorAll('.fm-ac-recent-chip'), function (c) {
            c.setAttribute('aria-pressed', 'false');
        });
    }

    function close() {
        if (backdrop) backdrop.classList.remove('fm-dlg-open');
    }

    function render(content) {
        ensureDOM();
        dialog.innerHTML = content;
        // Force reflow before adding class for transition
        void backdrop.offsetHeight;
        backdrop.classList.add('fm-dlg-open');
    }

    function escapeHtml(str) {
        var d = document.createElement('div');
        d.textContent = str;
        // Quotes too: several call sites drop the result into an attribute,
        // and textContent alone leaves those intact.
        return d.innerHTML.replace(/"/g, '&quot;').replace(/'/g, '&#39;');
    }

    function buildField(f) {
        var id = 'fm-dlg-f-' + f.name;
        var html = '<div class="fm-dlg-field">';
        html += '<label for="' + id + '">' + escapeHtml(f.label) + '</label>';

        if (f.type === 'autocomplete') {
            html += buildRecents(f, id);
            html += '<div class="fm-ac-wrap">';
            html += '<input id="' + id + '" type="text" autocomplete="off"'
                + (f.placeholder ? ' placeholder="' + escapeHtml(f.placeholder) + '"' : '')
                + '>';
            html += '<input type="hidden" id="' + id + '-val" name="' + f.name + '">';
            html += '<div class="fm-ac-list" id="' + id + '-list"></div>';
            html += '</div>';
        } else if (f.type === 'select') {
            html += '<select id="' + id + '" name="' + f.name + '">';
            if (f.placeholder) {
                html += '<option value="">' + escapeHtml(f.placeholder) + '</option>';
            }
            (f.options || []).forEach(function (o) {
                html += '<option value="' + escapeHtml(String(o.value)) + '">' + escapeHtml(o.text) + '</option>';
            });
            html += '</select>';
        } else if (f.type === 'number') {
            html += '<input id="' + id + '" name="' + f.name + '" type="number" min="0"'
                + (f.placeholder ? ' placeholder="' + escapeHtml(f.placeholder) + '"' : '')
                + (f.value !== undefined ? ' value="' + escapeHtml(String(f.value)) + '"' : '')
                + '>';
        } else {
            html += '<input id="' + id + '" name="' + f.name + '" type="text"'
                + (f.placeholder ? ' placeholder="' + escapeHtml(f.placeholder) + '"' : '')
                + (f.value !== undefined ? ' value="' + escapeHtml(String(f.value)) + '"' : '')
                + '>';
        }
        html += '</div>';
        return html;
    }

    /**
     * Recent picks for an autocomplete field, most recent first. Each one is a
     * button that fills the input below it. Nothing renders until there is
     * something to remember.
     */
    function buildRecents(f, id) {
        var items = fieldRecents(f);
        if (!items.length) return '';

        var html = '<div class="fm-ac-recent-row" id="' + id + '-recent">';
        if (f.recents.label) {
            html += '<span class="fm-ac-recent-tag">' + escapeHtml(f.recents.label) + '</span>';
        }
        items.forEach(function (item) {
            // The chip stays short — bare club name — while the field takes the
            // full "Name (Country)" the search results would have put there.
            html += '<button type="button" class="fm-ac-recent-chip" aria-pressed="false"'
                + ' data-value="' + escapeHtml(String(item.value)) + '"'
                + ' data-label="' + escapeHtml(item.label) + '"'
                + ' data-name="' + escapeHtml(item.name || item.label) + '"'
                + ' title="' + escapeHtml(item.label) + '">'
                + escapeHtml(item.name || item.label) + '</button>';
        });
        html += '</div>';
        return html;
    }

    function gatherData(fields) {
        var data = {};
        (fields || []).forEach(function (f) {
            if (f.type === 'autocomplete') {
                var hid = document.getElementById('fm-dlg-f-' + f.name + '-val');
                if (hid) data[f.name] = hid.value;
            } else {
                var el = document.getElementById('fm-dlg-f-' + f.name);
                if (el) data[f.name] = el.value;
            }
        });
        return data;
    }

    /**
     * A recent chip applies straight to the field: one click, value set,
     * search results put away.
     */
    function wireRecents(fieldId, input, hidden, list) {
        var row = document.getElementById(fieldId + '-recent');
        if (!row) return;

        row.addEventListener('click', function (e) {
            var chip = e.target.closest('.fm-ac-recent-chip');
            if (!chip) return;
            hidden.value = chip.dataset.value;
            hidden.dataset.label = chip.dataset.label;
            hidden.dataset.name = chip.dataset.name;
            input.value = chip.dataset.label;
            list.innerHTML = '';
            list.style.display = 'none';
            clearRecentSelection(row);
            chip.setAttribute('aria-pressed', 'true');
        });
    }

    /** Persist what was picked, so the next dialog can offer it back. */
    function rememberPicks(fields) {
        (fields || []).forEach(function (f) {
            if (f.type !== 'autocomplete' || !f.recents || !f.recents.key) return;
            var hid = document.getElementById('fm-dlg-f-' + f.name + '-val');
            if (!hid || !hid.value || !hid.dataset.label) return;
            writeRecent(f.recents.key, {
                value: hid.value,
                label: hid.dataset.label,
                name: hid.dataset.name || hid.dataset.label,
            });
        });
    }

    /**
     * Open a form dialog.
     */
    function open(opts) {
        var html = '<div class="fm-dlg-header">' + escapeHtml(opts.title || 'Dialog') + '</div>';
        html += '<div class="fm-dlg-body">';

        if (opts.message) {
            html += '<p class="fm-dlg-msg">' + escapeHtml(opts.message) + '</p>';
        }

        (opts.fields || []).forEach(function (f) {
            html += buildField(f);
        });

        html += '</div>';
        html += '<div class="fm-dlg-actions">';
        html += '<button class="fm-dlg-btn fm-dlg-cancel">' + escapeHtml(opts.cancelText || 'Cancel') + '</button>';
        html += '<button class="fm-dlg-btn fm-dlg-confirm">' + escapeHtml(opts.confirmText || 'OK') + '</button>';
        html += '</div>';

        render(html);

        // Wire up autocomplete fields
        (opts.fields || []).forEach(function (f) {
            if (f.type !== 'autocomplete') return;
            var fieldId = 'fm-dlg-f-' + f.name;
            var input = document.getElementById(fieldId);
            var hidden = document.getElementById(fieldId + '-val');
            var list = document.getElementById(fieldId + '-list');
            if (!input || !hidden || !list) return;

            wireRecents(fieldId, input, hidden, list);

            var recentRow = document.getElementById(fieldId + '-recent');

            var timer = null;
            input.addEventListener('input', function () {
                clearTimeout(timer);
                clearRecentSelection(recentRow);
                hidden.value = '';
                delete hidden.dataset.label;
                delete hidden.dataset.name;
                var q = input.value.trim();
                if (q.length < 1) { list.innerHTML = ''; list.style.display = 'none'; return; }
                timer = setTimeout(function () {
                    var base = f.url || '/api/clubs';
                    var sep = base.indexOf('?') >= 0 ? '&' : '?';
                    var url = base + sep + 'q=' + encodeURIComponent(q);
                    fetch(url).then(function (r) { return r.json(); }).then(function (items) {
                        if (!items.length) { list.innerHTML = ''; list.style.display = 'none'; return; }
                        list.innerHTML = items.slice(0, 20).map(function (item) {
                            var label = item.country
                                ? item.name + ' (' + item.country + ')'
                                : item.name;
                            return '<div class="fm-ac-item" data-value="' + escapeHtml(String(item.id)) + '"'
                                + ' data-name="' + escapeHtml(item.name) + '">'
                                + escapeHtml(label) + '</div>';
                        }).join('');
                        list.style.display = 'block';
                    });
                }, 200);
            });

            list.addEventListener('click', function (e) {
                var item = e.target.closest('.fm-ac-item');
                if (!item) return;
                hidden.value = item.dataset.value;
                hidden.dataset.label = item.textContent; // shows "Name (Country)"
                hidden.dataset.name = item.dataset.name;
                input.value = item.textContent;
                list.innerHTML = '';
                list.style.display = 'none';
                clearRecentSelection(recentRow);
            });
        });

        dialog.querySelector('.fm-dlg-cancel').addEventListener('click', close);
        dialog.querySelector('.fm-dlg-confirm').addEventListener('click', function () {
            var data = gatherData(opts.fields);
            rememberPicks(opts.fields);
            if (opts.onConfirm) opts.onConfirm(data);
            close();
        });

        // Focus first input
        var first = dialog.querySelector('input, select');
        if (first) first.focus();
    }

    /**
     * Simple confirm dialog (title + message + buttons).
     */
    function confirm(opts) {
        open({
            title: opts.title,
            message: opts.message,
            fields: [],
            confirmText: opts.confirmText || 'Confirm',
            cancelText: opts.cancelText || 'Cancel',
            onConfirm: opts.onConfirm,
        });
    }

    window.OpenFootballDialog = { open: open, confirm: confirm, close: close };
})();
