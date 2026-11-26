package app.nearvolt

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.OpenableColumns
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * File access for the Rust engine. Picked files are remembered under a token,
 * and the engine opens each one through [openSource] only when it reaches it.
 */
object NativeBridge {
    private lateinit var resolver: ContentResolver
    private val selections = ConcurrentHashMap<Long, List<Uri>>()
    private val nextToken = AtomicLong(1)

    fun register(context: Context) {
        if (::resolver.isInitialized) return
        resolver = context.contentResolver
        System.loadLibrary("nearvolt_ffi")
        register(this)
    }

    @JvmStatic
    private external fun register(bridge: NativeBridge)

    /** Called by the engine. Returns a descriptor the engine now owns, or -1. */
    fun openSource(token: Long, index: Int): Int = try {
        val uri = selections[token]?.getOrNull(index) ?: return -1
        resolver.openFileDescriptor(uri, "r")?.detachFd() ?: -1
    } catch (e: Exception) {
        -1
    }

    /** Describes the picked documents and remembers them for the engine. */
    fun fromDocuments(uris: List<Uri>): PickResult? {
        val files = mutableListOf<PickedFile>()
        val kept = mutableListOf<Uri>()
        for (uri in uris) {
            val file = resolver.query(uri, null, null, null, null)?.use { cursor ->
                if (!cursor.moveToFirst()) return@use null
                PickedFile(
                    path = cursor.string(OpenableColumns.DISPLAY_NAME) ?: "file",
                    size = cursor.long(OpenableColumns.SIZE) ?: return@use null,
                    modifiedMs = cursor.long(Document.COLUMN_LAST_MODIFIED),
                )
            } ?: continue
            files += file
            kept += uri
        }
        return remember(files, kept)
    }

    /** Walks a picked folder. Paths start with the folder's own name. */
    fun fromTree(tree: Uri): PickResult? {
        resolver.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        val rootId = DocumentsContract.getTreeDocumentId(tree)
        val rootName = resolver.query(DocumentsContract.buildDocumentUriUsingTree(tree, rootId), null, null, null, null)
            ?.use { if (it.moveToFirst()) it.string(Document.COLUMN_DISPLAY_NAME) else null } ?: "Folder"
        val files = mutableListOf<PickedFile>()
        val uris = mutableListOf<Uri>()
        val pending = ArrayDeque(listOf(rootId to rootName))
        val columns = arrayOf(
            Document.COLUMN_DOCUMENT_ID,
            Document.COLUMN_DISPLAY_NAME,
            Document.COLUMN_MIME_TYPE,
            Document.COLUMN_SIZE,
            Document.COLUMN_LAST_MODIFIED,
        )
        while (pending.isNotEmpty()) {
            val (docId, path) = pending.removeFirst()
            val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, docId)
            resolver.query(children, columns, null, null, null)?.use { cursor ->
                while (cursor.moveToNext()) {
                    val id = cursor.getString(0)
                    val name = "$path/${cursor.getString(1)}"
                    if (cursor.getString(2) == Document.MIME_TYPE_DIR) {
                        pending += id to name
                    } else {
                        files += PickedFile(name, cursor.getLong(3), if (cursor.isNull(4)) null else cursor.getLong(4))
                        uris += DocumentsContract.buildDocumentUriUsingTree(tree, id)
                    }
                }
            }
        }
        return remember(files, uris)
    }

    private fun remember(files: List<PickedFile>, uris: List<Uri>): PickResult? {
        if (files.isEmpty()) return null
        val token = nextToken.getAndIncrement()
        selections[token] = uris
        return PickResult(token, files)
    }

    private fun Cursor.string(column: String): String? =
        getColumnIndex(column).takeIf { it >= 0 && !isNull(it) }?.let(::getString)

    private fun Cursor.long(column: String): Long? =
        getColumnIndex(column).takeIf { it >= 0 && !isNull(it) }?.let(::getLong)
}
