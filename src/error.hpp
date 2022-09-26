#pragma once

#include <QString>

#include <cstdlib>

struct error;

extern "C" {
    void error_free(error *err);
    char *error_message(const error *err);
}

class Error final {
public:
    Error() : m_obj(nullptr) {}
    explicit Error(error *obj) : m_obj(obj) {}
    Error(const Error &) = delete;
    ~Error()
    {
        if (m_obj) {
            error_free(m_obj);
        }
    }

public:
    Error &operator=(const Error &) = delete;
    error **operator&() { return &m_obj; }
    operator bool() const { return m_obj != nullptr; }

public:
    // The caller must check if this error has a value before calling this method.
    QString message() const
    {
        auto msg = error_message(m_obj);
        QString copied(msg);
        std::free(msg);
        return copied;
    }

private:
    error *m_obj;
};
