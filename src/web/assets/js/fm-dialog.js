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
 */
(function () {
    'use strict';

    let backdrop = null;
    let dialog = null;

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
        return d.innerHTML;
    }

    function buildField(f) {
        var id = 'fm-dlg-f-' + f.name;
        var html = '<div class="fm-dlg-field">';
        html += '<label for="' + id + '">' + escapeHtml(f.label) + '</label>';

        if (f.type === 'autocomplete') {
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

            var timer = null;
            input.addEventListener('input', function () {
                clearTimeout(timer);
                hidden.value = '';
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
                input.value = item.textContent; // shows "Name (Country)"
                list.innerHTML = '';
                list.style.display = 'none';
            });
        });

        dialog.querySelector('.fm-dlg-cancel').addEventListener('click', close);
        dialog.querySelector('.fm-dlg-confirm').addEventListener('click', function () {
            var data = gatherData(opts.fields);
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
