// Reads the clipboard through Qt, in-process. Shelling out to wl-paste on the
// GUI thread deadlocks when this app owns the clipboard: the compositor asks
// us to serve the data while our event loop is blocked waiting for wl-paste.
#pragma once

#include <QtCore/QByteArray>
#include <QtCore/QMimeData>
#include <QtCore/QStringList>
#include <QtGui/QClipboard>
#include <QtGui/QGuiApplication>

#include "rust/cxx.h"

inline const QMimeData* hermes_clipboard_mime()
{
    QClipboard* clipboard = QGuiApplication::clipboard();
    return clipboard ? clipboard->mimeData(QClipboard::Clipboard) : nullptr;
}

inline rust::Vec<rust::String> clipboard_formats()
{
    rust::Vec<rust::String> out;
    if (const QMimeData* mime = hermes_clipboard_mime()) {
        for (const QString& format : mime->formats())
            out.push_back(rust::String(format.toStdString()));
    }
    return out;
}

inline rust::Vec<std::uint8_t> clipboard_data(rust::Str format)
{
    rust::Vec<std::uint8_t> out;
    if (const QMimeData* mime = hermes_clipboard_mime()) {
        const QByteArray bytes = mime->data(QString::fromUtf8(format.data(), format.size()));
        out.reserve(bytes.size());
        for (char byte : bytes)
            out.push_back(static_cast<std::uint8_t>(byte));
    }
    return out;
}

inline rust::String clipboard_text()
{
    if (const QMimeData* mime = hermes_clipboard_mime(); mime && mime->hasText())
        return rust::String(mime->text().toStdString());
    return rust::String();
}
