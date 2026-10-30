import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ProgressReporter } from './progress';

describe('ProgressReporter', () => {
	const onError = (error: unknown) => {
		throw error;
	};

	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it('sends only the last page of a burst, stamped with its turn time', async () => {
		const send = vi.fn().mockResolvedValue({ accepted: true });
		const reporter = new ProgressReporter(7, onError, send, 1000);
		reporter.turn(1, 100);
		reporter.turn(2, 200);
		reporter.turn(3, 300);
		expect(send).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1000);
		expect(send).toHaveBeenCalledExactlyOnceWith(7, { page: 3, client_time: 300 }, { keepalive: false });
	});

	it('flushes immediately with keepalive and does not resend afterwards', async () => {
		const send = vi.fn().mockResolvedValue({ accepted: true });
		const reporter = new ProgressReporter(7, onError, send, 1000);
		reporter.turn(4, 400);
		await reporter.flush(true);
		expect(send).toHaveBeenCalledExactlyOnceWith(7, { page: 4, client_time: 400 }, { keepalive: true });
		await vi.advanceTimersByTimeAsync(1000);
		await reporter.flush(true);
		expect(send).toHaveBeenCalledOnce();
	});

	it('passes failures of a direct flush to the caller', async () => {
		const send = vi.fn().mockRejectedValue(new Error('offline'));
		const reporter = new ProgressReporter(7, onError, send, 1000);
		reporter.turn(1, 100);
		await expect(reporter.flush()).rejects.toThrow('offline');
	});

	it('reports failures of a timed send through onError', async () => {
		const error = new Error('offline');
		const failing = vi.fn();
		const reporter = new ProgressReporter(7, failing, vi.fn().mockRejectedValue(error), 1000);
		reporter.turn(1, 100);
		await vi.advanceTimersByTimeAsync(1000);
		expect(failing).toHaveBeenCalledExactlyOnceWith(error);
	});
});
