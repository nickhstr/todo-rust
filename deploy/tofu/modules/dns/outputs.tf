output "record_names" {
  value = [for r in cloudflare_record.app : r.hostname]
}
