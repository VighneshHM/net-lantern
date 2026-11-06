import { useMutation, useQueryClient } from "@tanstack/react-query";
import { apiFetch } from "./client";
import { ApiRequestError, type Device } from "./queries";

export interface DevicePatch {
  name?: string | null;
  starred?: boolean;
  is_new?: false;
}

/** Change a device's name, star or "new" flag. Throws `ApiRequestError` on failure. */
export async function updateDevice(id: number, patch: DevicePatch): Promise<Device> {
  const res = await apiFetch<Device>(`/api/devices/${id}`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });
  if (res.ok) return res.data;
  throw new ApiRequestError(res.status, res.error);
}

interface Snapshot {
  list: Device[] | undefined;
  one: Device | undefined;
}

/**
 * Update a device, showing the change at once in the list and the single-device
 * cache, restoring both if the server refuses, and trusting the server's reply
 * when it accepts.
 */
export function useUpdateDevice() {
  const qc = useQueryClient();
  return useMutation<Device, Error, { id: number; patch: DevicePatch }, Snapshot>({
    mutationFn: ({ id, patch }) => updateDevice(id, patch),
    onMutate: async ({ id, patch }) => {
      await qc.cancelQueries({ queryKey: ["devices"] });
      await qc.cancelQueries({ queryKey: ["device", id] });
      const snap: Snapshot = {
        list: qc.getQueryData<Device[]>(["devices"]),
        one: qc.getQueryData<Device>(["device", id]),
      };
      const apply = (d: Device): Device => ({
        ...d,
        ...(patch.starred !== undefined && { starred: patch.starred }),
        ...(patch.is_new === false && { is_new: false }),
        ...(patch.name !== undefined && {
          name: patch.name,
          display_name: patch.name ? patch.name : d.display_name,
        }),
      });
      qc.setQueryData<Device[]>(["devices"], (old) =>
        old?.map((d) => (d.id === id ? apply(d) : d)),
      );
      qc.setQueryData<Device>(["device", id], (old) => (old ? apply(old) : old));
      return snap;
    },
    onError: (_e, { id }, snap) => {
      if (!snap) return;
      // Restore only this device: other rows may have changed since the snapshot.
      const before = snap.list?.find((d) => d.id === id);
      if (before) {
        qc.setQueryData<Device[]>(["devices"], (old) =>
          old?.map((d) => (d.id === id ? before : d)),
        );
      }
      qc.setQueryData(["device", id], snap.one);
    },
    onSuccess: (device, { id }) => {
      qc.setQueryData<Device[]>(["devices"], (old) =>
        old?.map((d) => (d.id === id ? device : d)),
      );
      qc.setQueryData<Device>(["device", id], device);
    },
    // A refetch that started mid-request may hold stale data; settle on the server's truth.
    onSettled: (_d, _e, { id }) => {
      void qc.invalidateQueries({ queryKey: ["devices"] });
      void qc.invalidateQueries({ queryKey: ["device", id] });
    },
  });
}
