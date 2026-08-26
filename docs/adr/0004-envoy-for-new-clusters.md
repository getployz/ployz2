# Use Envoy for Public Ingress on new Clusters

New Clusters use Envoy as their immutable Ingress Proxy backend, while existing Caddy Clusters remain on Caddy. Envoy provides the native routing, passive failure handling, and request evidence Ployz needs without replacing the product's observer-relative model. The first production contract keeps file-watched xDS and round-robin routing; live backend migration, ADS, regional or header routing, custom balancing, telemetry storage, and an administration surface wait for demonstrated need.
