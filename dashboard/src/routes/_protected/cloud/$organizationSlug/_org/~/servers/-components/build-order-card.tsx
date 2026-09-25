import { Card, CardAction, CardDescription, CardHeader, CardTitle } from "#/components/ui/card";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "#/components/ui/select";
import { BUILD_ORDERS, BUILD_ORDER_LABELS } from "#/modules/deployments/build-order";
import { useBuildOrder } from "#/modules/deployments/build-order.collection";

/** Where the Organization's Image Builds run. Takes effect on the next build; never staged. */
export function BuildOrderCard({ organizationSlug }: { organizationSlug: string }) {
  const { buildOrder, setBuildOrder } = useBuildOrder(organizationSlug);
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>Builds</CardTitle>
        <CardDescription>Where new image builds run</CardDescription>
        <CardAction>
          <Select value={buildOrder} onValueChange={(next) => {
            const order = BUILD_ORDERS.find((candidate) => candidate === next);
            if (order) setBuildOrder(order);
          }}>
            <SelectTrigger aria-label="Build on" className="w-56">
              <SelectValue>{BUILD_ORDER_LABELS[buildOrder]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {BUILD_ORDERS.map((order) => (
                  <SelectItem key={order} value={order} label={BUILD_ORDER_LABELS[order]}>{BUILD_ORDER_LABELS[order]}</SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </CardAction>
      </CardHeader>
    </Card>
  );
}
