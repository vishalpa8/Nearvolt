package app.nearvolt

import android.os.SystemClock
import java.lang.reflect.InvocationTargetException
import java.lang.reflect.Proxy
import kotlin.coroutines.Continuation

/**
 * Writes every call the app makes into Android to the log: what was asked, how it
 * ended and how long it took. Used while the app is in development, so a screen that
 * "does nothing" can be traced to the call that never came back.
 */
object CallLog {
    /** Too frequent or too self-referential to be worth a line each. */
    private val quiet = setOf("log", "updateTransferService")

    fun wrap(target: NearvoltPlatform): NearvoltPlatform {
        val handler = java.lang.reflect.InvocationHandler { _, method, args ->
            val values = args ?: emptyArray()
            if (method.name in quiet) return@InvocationHandler invoke(target, method, values)

            val shown = values.filter { it !is Continuation<*> }.joinToString(", ") { describe(it) }
            AppLog.write("call", "${method.name}($shown)")
            val started = SystemClock.elapsedRealtime()
            fun took() = "${SystemClock.elapsedRealtime() - started} ms"

            val last = values.lastOrNull()
            if (last is Continuation<*>) {
                // A call that waits (for a prompt, a scan, a download): its answer comes later.
                @Suppress("UNCHECKED_CAST")
                val original = last as Continuation<Any?>
                values[values.lastIndex] = object : Continuation<Any?> {
                    override val context = original.context
                    override fun resumeWith(result: Result<Any?>) {
                        result
                            .onSuccess { AppLog.write("call", "${method.name} answered ${describe(it)} after ${took()}") }
                            .onFailure { AppLog.write("call", "${method.name} failed after ${took()}: $it") }
                        original.resumeWith(result)
                    }
                }
                invoke(target, method, values, onError = { AppLog.write("call", "${method.name} failed at once: $it") })
            } else {
                invoke(target, method, values, onError = { AppLog.write("call", "${method.name} failed: $it") }).also {
                    if (method.returnType != Void.TYPE) AppLog.write("call", "${method.name} returned ${describe(it)} in ${took()}")
                }
            }
        }
        return Proxy.newProxyInstance(
            NearvoltPlatform::class.java.classLoader,
            arrayOf(NearvoltPlatform::class.java),
            handler,
        ) as NearvoltPlatform
    }

    private fun invoke(
        target: Any,
        method: java.lang.reflect.Method,
        args: Array<Any?>,
        onError: (Throwable) -> Unit = {},
    ): Any? = try {
        method.invoke(target, *args)
    } catch (e: InvocationTargetException) {
        onError(e.targetException)
        throw e.targetException
    }

    /**
     * What to write about a value. Text is only counted (it can be a code or a name),
     * and results are described by kind, so the log stays short and holds no secrets.
     */
    internal fun describe(value: Any?): String = when (value) {
        null -> "null"
        is Unit -> "done"
        is Boolean, is Number, is Enum<*> -> value.toString()
        is String -> "text(${value.length})"
        is Collection<*> -> "${value.size} items"
        is Readiness -> value.toString()
        else -> value.javaClass.simpleName
    }
}
