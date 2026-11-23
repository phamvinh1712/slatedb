package io.slatedb.uniffi;

import static org.junit.jupiter.api.Assertions.assertTrue;

import java.lang.ref.Reference;

import org.junit.jupiter.api.Test;

class SlateDbMemoryTest {
    @Test
    void countsNativeBytesHeldByWriteBatch() throws Exception {
        byte[] value = new byte[8 * 1024 * 1024];
        long before = Slatedb.nativeAllocatedBytes();
        long retained;

        try (WriteBatch batch = new WriteBatch()) {
            batch.put(new byte[] {1}, value);
            retained = Slatedb.nativeAllocatedBytes();
            assertTrue(retained - before >= value.length, "Expected the batch to retain its value");
        }

        long after = Slatedb.nativeAllocatedBytes();
        assertTrue(retained - after >= value.length, "Expected the batch to release its value");
        Reference.reachabilityFence(value);
    }
}
