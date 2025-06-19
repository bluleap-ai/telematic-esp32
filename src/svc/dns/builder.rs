use heapless::Vec;

pub struct DnsBuilder {
    raw: heapless::Vec<u8, 80>,
}

impl DnsBuilder {
    pub fn build(domain: &str) -> Self {
        let mut query: Vec<u8, 80> = Vec::new();

        // Header
        let _ = query.extend_from_slice(&[
            0xAB, 0xCD, // Transaction ID (arbitrary)
            0x01, 0x00, // Flags: standard query
            0x00, 0x01, // Questions: 1
            0x00, 0x00, // Answer RRs: 0
            0x00, 0x00, // Authority RRs: 0
            0x00, 0x00, // Additional RRs: 0
        ]);

        // Question
        for part in domain.split('.') {
            let _ = query.push(part.len() as u8); // Label length
            let _ = query.extend_from_slice(part.as_bytes()); // Label
        }
        let _ = query.push(0); // End of domain name
        let _ = query.extend_from_slice(&[
            0x00, 0x01, // Type: A (IPv4 address)
            0x00, 0x01, // Class: IN (Internet)
        ]);

        Self { raw: query }
    }

    pub fn query_data(mut self) -> heapless::Vec<u8, 80> {
        let length = self.raw.len();
        let _ = self.raw.insert(0, (length & 0xFF) as u8);
        let _ = self.raw.insert(0, (length >> 8) as u8);
        self.raw
    }

    pub fn parse_dns_response(response: &[u8]) -> Result<[u8; 4], ()> {
        let mut ips: [u8; 4] = [0u8; 4];

        // Skip the header (12 bytes) and question section
        let mut idx = 12;
        while response[idx] != 0 {
            idx += 1 + response[idx] as usize; // Skip each label
        }
        idx += 5; // Skip null byte and QTYPE/QCLASS

        // Parse the answer section
        while idx < response.len() {
            idx += 10; // Skip name, type, class, and TTL
            let data_len = (response[idx] as usize) << 8 | response[idx + 1] as usize;
            idx += 2;
            if data_len == 4 {
                // IPv4 address
                ips[0] = response[idx];
                ips[1] = response[idx + 1];
                ips[2] = response[idx + 2];
                ips[3] = response[idx + 3];
                return Ok(ips);
            }
            idx += data_len;
        }

        Err(())
    }
}
